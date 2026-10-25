import Foundation

/// A small, plain-text campaign. Remote content can choose a native symbol
/// and a near.ai destination, never supply executable or vector artwork.
public struct PrivateAICampaign: Sendable, Equatable, Decodable {
    public let schemaVersion: Int
    public let id: String
    public let title: String
    public let body: String
    public let actionTitle: String
    public let actionURL: URL
    public let symbol: String
    public let imageURL: URL?

    private enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version", id, title, body, symbol
        case actionTitle = "action_title", actionURL = "action_url", imageURL = "image_url"
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = try values.decode(Int.self, forKey: .schemaVersion)
        id = try values.decode(String.self, forKey: .id)
        title = try values.decode(String.self, forKey: .title)
        body = try values.decode(String.self, forKey: .body)
        actionTitle = try values.decode(String.self, forKey: .actionTitle)
        actionURL = try values.decode(URL.self, forKey: .actionURL)
        symbol = try values.decode(String.self, forKey: .symbol)
        imageURL = try values.decodeIfPresent(URL.self, forKey: .imageURL)
        guard schemaVersion == 1,
              Self.plain(id, limit: 100), Self.plain(title, limit: 100), Self.plain(body, limit: 300),
              Self.plain(actionTitle, limit: 40), ["cpu", "sparkles", "bolt", "globe"].contains(symbol),
              Self.secureURL(actionURL), actionURL.host?.lowercased() == "near.ai",
              imageURL.map({ Self.rasterURL($0) }) ?? true else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "invalid campaign"))
        }
    }

    public static func decode(from data: Data, endpoint: URL? = nil) -> Self? {
        guard data.count <= PrivateAICampaignClient.manifestLimit,
              let campaign = try? JSONDecoder().decode(Self.self, from: data) else { return nil }
        if let image = campaign.imageURL, let endpoint, !imageAllowed(image, endpoint: endpoint) { return nil }
        return campaign
    }

    private static func plain(_ value: String, limit: Int) -> Bool {
        !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && value.count <= limit
            && value.rangeOfCharacter(from: .controlCharacters) == nil
            && !value.contains("<") && !value.contains(">")
    }

    static func secureURL(_ url: URL) -> Bool {
        guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return false }
        return parts.scheme?.lowercased() == "https" && !(parts.host ?? "").isEmpty
            && parts.user == nil && parts.password == nil && (parts.port == nil || parts.port == 443)
            && parts.fragment == nil
    }

    private static func rasterURL(_ url: URL) -> Bool {
        secureURL(url) && ["png", "jpg", "jpeg"].contains(url.pathExtension.lowercased()) && safePath(url) != nil
    }

    /// Use decoded, unambiguous path components: encoded separators, dot
    /// segments and double escaping must not give the server another path.
    private static func safePath(_ url: URL) -> String? {
        guard let encoded = URLComponents(url: url, resolvingAgainstBaseURL: false)?.percentEncodedPath,
              !encoded.lowercased().contains("%2f"), !encoded.lowercased().contains("%5c"),
              let path = encoded.removingPercentEncoding, !path.contains("%"), !path.contains("\\"),
              path.rangeOfCharacter(from: .controlCharacters) == nil,
              !path.split(separator: "/", omittingEmptySubsequences: false).contains(where: { $0 == "." || $0 == ".." })
        else { return nil }
        return path
    }

    static func imageAllowed(_ image: URL, endpoint: URL) -> Bool {
        guard secureURL(endpoint), rasterURL(image),
              image.host?.lowercased() == endpoint.host?.lowercased(),
              let imagePath = safePath(image), let manifestPath = safePath(endpoint) else { return false }
        return (imagePath as NSString).deletingLastPathComponent == (manifestPath as NSString).deletingLastPathComponent
    }
}
