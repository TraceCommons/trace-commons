import Foundation

/// Each coding tool's setup guide (`tc_install_guides_json`, the core's
/// `install_guides`): the one table first run and managed sessions read
/// (Ron's review of #1419, 2026-10-15). Keyed by the discovery slug.
public struct InstallGuides: Equatable, Sendable {
    public let urls: [String: String]

    public init(urls: [String: String]) { self.urls = urls }

    /// Empty for nil or unreadable input: no Install button rather than a
    /// guessed link.
    public init(json: String?) {
        let decoded = json.flatMap { try? JSONDecoder().decode([String: String].self, from: Data($0.utf8)) }
        self.init(urls: decoded ?? [:])
    }

    /// The guide for `kind`, over https only; nil when the core gave none.
    public func url(for kind: SourceKind) -> URL? {
        guard let raw = urls[kind.rawValue], let url = URL(string: raw), url.scheme == "https" else { return nil }
        return url
    }
}
