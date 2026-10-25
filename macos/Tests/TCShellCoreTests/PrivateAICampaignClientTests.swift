import Foundation
import XCTest

@testable import TCShellCore

final class PrivateAICampaignClientTests: XCTestCase {
    private let endpoint = URL(string: "https://assets.example/native/campaign.json")!
    private let manifest = Data(#"{"schema_version":1,"id":"kimi","title":"Kimi K3.1 Pro","body":"Try Kimi with Private AI.","action_title":"Try Kimi","action_url":"https://near.ai/models/kimi","symbol":"cpu","image_url":"https://assets.example/native/kimi.png"}"#.utf8)

    private func response(_ data: Data, status: Int = 200, mime: String = "application/json", length: Int? = nil,
                          url: URL? = nil) throws -> PrivateAICampaignClient.Response {
        var headers = ["Content-Type": mime]
        if let length { headers["Content-Length"] = String(length) }
        return try .init(data: data, response: XCTUnwrap(HTTPURLResponse(
            url: url ?? endpoint, statusCode: status, httpVersion: nil, headerFields: headers)))
    }

    func testRemoteSuccessUsesAnAnonymousFiveSecondRequestAndValidatesArtwork() async throws {
        let reply = try response(manifest)
        let client = PrivateAICampaignClient { request, limit in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(request.timeoutInterval, 5)
            XCTAssertFalse(request.httpShouldHandleCookies)
            XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
            XCTAssertNil(request.value(forHTTPHeaderField: "Cookie"))
            XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "application/json")
            XCTAssertEqual(limit, 16 * 1024)
            return reply
        }
        let campaign = try await client.fetch(from: endpoint)
        XCTAssertEqual(campaign.title, "Kimi K3.1 Pro")
        XCTAssertEqual(campaign.imageURL?.lastPathComponent, "kimi.png")
    }

    func testSessionHasNoPersistentCredentialsCookiesOrCache() {
        let config = PrivateAICampaignClient.configuration()
        XCTAssertNil(config.httpCookieStorage)
        XCTAssertNil(config.urlCredentialStorage)
        XCTAssertNil(config.urlCache)
        XCTAssertFalse(config.httpShouldSetCookies)
        XCTAssertEqual(config.timeoutIntervalForRequest, 5)
        XCTAssertEqual(config.timeoutIntervalForResource, 5)
    }

    func testRedirectHTTPFailureWrongMIMEAndChangedFinalURLAreRejected() async throws {
        for reply in try [response(manifest, status: 302), response(manifest, status: 500),
                          response(manifest, mime: "text/html"),
                          response(manifest, url: URL(string: "https://other.example/campaign.json")!)] {
            let client = PrivateAICampaignClient { _, _ in reply }
            do {
                _ = try await client.fetch(from: endpoint)
                XCTFail("invalid remote response accepted")
            } catch { XCTAssertEqual(error as? PrivateAICampaignClient.Failure, .invalidResponse) }
        }
        let session = URLSession(configuration: .ephemeral)
        defer { session.invalidateAndCancel() }
        let redirect = try XCTUnwrap(HTTPURLResponse(url: endpoint, statusCode: 302, httpVersion: nil, headerFields: nil))
        let request = URLRequest(url: URL(string: "https://other.example/tracking")!)
        let followed = await PrivateAICampaignSessionDelegate().urlSession(
            session, task: session.dataTask(with: endpoint), willPerformHTTPRedirection: redirect, newRequest: request)
        XCTAssertNil(followed, "even same-origin redirects must not be followed")
    }

    func testMalformedAndOversizedManifestsFailWithoutReplacingFallback() async throws {
        for (reply, expected) in try [
            (response(Data("{}".utf8)), PrivateAICampaignClient.Failure.invalidCampaign),
            (response(Data(repeating: 0x20, count: 16 * 1024 + 1)), .tooLarge),
            (response(manifest, length: 16 * 1024 + 1), .tooLarge),
        ] {
            let client = PrivateAICampaignClient { _, _ in reply }
            do { _ = try await client.fetch(from: endpoint); XCTFail("invalid manifest accepted") }
            catch { XCTAssertEqual(error as? PrivateAICampaignClient.Failure, expected) }
        }
        let client = PrivateAICampaignClient { _, _ in throw URLError(.timedOut) }
        do { _ = try await client.fetch(from: endpoint); XCTFail("transport failure accepted") }
        catch { XCTAssertEqual((error as? URLError)?.code, .timedOut) }
    }

    func testUntrustedEndpointIsRejectedBeforeTransport() async {
        let client = PrivateAICampaignClient { _, _ in
            XCTFail("transport invoked for an insecure endpoint")
            throw URLError(.badURL)
        }
        do { _ = try await client.fetch(from: URL(string: "http://assets.example/card.json")!); XCTFail("HTTP accepted") }
        catch { XCTAssertEqual(error as? PrivateAICampaignClient.Failure, .invalidEndpoint) }
    }

    func testRasterMIMEAndMagicMustAgreeAndImageBytesAreBounded() async throws {
        let campaign = try XCTUnwrap(PrivateAICampaign.decode(from: manifest, endpoint: endpoint))
        let imageURL = try XCTUnwrap(campaign.imageURL)
        let png = Data([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
        let valid = try response(png, mime: "image/png", url: imageURL)
        let good = PrivateAICampaignClient { request, limit in
            XCTAssertEqual(request.url, imageURL)
            XCTAssertEqual(limit, 2 * 1024 * 1024)
            return valid
        }
        let loaded = try await good.fetchImage(for: campaign, from: endpoint)
        XCTAssertEqual(loaded, png)
        for (reply, expected) in try [
            (response(Data("<svg/>".utf8), mime: "image/png", url: imageURL), PrivateAICampaignClient.Failure.invalidImage),
            (response(png, mime: "image/svg+xml", url: imageURL), .invalidImage),
            (response(png, mime: "image/jpeg", url: imageURL), .invalidImage),
            (response(png, mime: "image/png", length: 2 * 1024 * 1024 + 1, url: imageURL), .tooLarge),
            (response(Data(repeating: 0, count: 2 * 1024 * 1024 + 1), mime: "image/png", url: imageURL), .tooLarge),
        ] {
            let client = PrivateAICampaignClient { _, _ in reply }
            do { _ = try await client.fetchImage(for: campaign, from: endpoint); XCTFail("invalid image accepted") }
            catch { XCTAssertEqual(error as? PrivateAICampaignClient.Failure, expected) }
        }
        do {
            _ = try await good.fetchImage(for: campaign, from: URL(string: "https://other.example/native/campaign.json")!)
            XCTFail("image accepted for another endpoint")
        } catch { XCTAssertEqual(error as? PrivateAICampaignClient.Failure, .invalidImage) }
    }

    func testStreamingReaderStopsAtLimitPlusOneWithoutDrainingAnUnboundedBody() async throws {
        let source = CampaignByteSource()
        let bytes = AsyncStream<UInt8>(unfolding: { await source.next() })
        do { _ = try await PrivateAICampaignClient.read(bytes, limit: 3); XCTFail("oversized stream accepted") }
        catch { XCTAssertEqual(error as? PrivateAICampaignClient.Failure, .tooLarge) }
        let consumed = await source.consumed
        XCTAssertEqual(consumed, 4)
        let exact = AsyncStream<UInt8> { continuation in
            for byte: UInt8 in [1, 2, 3] { continuation.yield(byte) }
            continuation.finish()
        }
        let data = try await PrivateAICampaignClient.read(exact, limit: 3)
        XCTAssertEqual(data, Data([1, 2, 3]))
    }

    func testJPEGArtworkAndAnAbsentImageAreSupported() async throws {
        let jpegManifest = Data(String(decoding: manifest, as: UTF8.self).replacingOccurrences(of: "kimi.png", with: "kimi.jpg").utf8)
        let campaign = try XCTUnwrap(PrivateAICampaign.decode(from: jpegManifest, endpoint: endpoint))
        let jpeg = Data([0xff, 0xd8, 0xff, 0xe0])
        let reply = try response(jpeg, mime: "image/jpeg", url: campaign.imageURL)
        let client = PrivateAICampaignClient { _, _ in reply }
        let loaded = try await client.fetchImage(for: campaign, from: endpoint)
        XCTAssertEqual(loaded, jpeg)

        var fields = try XCTUnwrap(JSONSerialization.jsonObject(with: manifest) as? [String: Any])
        fields.removeValue(forKey: "image_url")
        let noImage = try XCTUnwrap(PrivateAICampaign.decode(from: JSONSerialization.data(withJSONObject: fields)))
        let unused = PrivateAICampaignClient { _, _ in
            XCTFail("an absent image must not trigger a request")
            throw URLError(.badURL)
        }
        let absent = try await unused.fetchImage(for: noImage, from: endpoint)
        XCTAssertNil(absent)
    }

    func testCancellationPropagatesDuringAFetch() async throws {
        let reply = try response(manifest)
        let (started, signal) = AsyncStream<Void>.makeStream()
        let client = PrivateAICampaignClient { _, _ in
            signal.yield(())
            try await Task.sleep(for: .seconds(30))
            return reply
        }
        let endpoint = self.endpoint
        let task = Task { try await client.fetch(from: endpoint) }
        var iterator = started.makeAsyncIterator()
        _ = await iterator.next()
        task.cancel()
        do { _ = try await task.value; XCTFail("cancelled fetch succeeded") }
        catch { XCTAssertTrue(error is CancellationError) }
        signal.finish()
    }
}

private actor CampaignByteSource {
    private(set) var consumed = 0
    func next() -> UInt8? { consumed += 1; return 0 }
}
