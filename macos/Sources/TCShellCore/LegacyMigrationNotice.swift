import Foundation

/// `status.legacy_invite_migration`: whether moving a legacy invite identity
/// to a NEAR AI account is offered, and the notice after it moved.
///
/// Only the notice is read here, and only to hand it back to the Rust
/// (`TCConsentCopy.legacyMigrationNoticeJSON`), which reads `folders_kept`
/// and returns the words. This shell never reads it to choose a sentence.
/// The offer itself is made in the Tauri app, not here.
///
/// A daemon that predates the field, or a notice that is not an object,
/// reads as nothing to show.
public struct LegacyMigrationWire: Decodable, Equatable, Sendable {
    /// The notice object as the daemon sent it, re-encoded, or nil when there
    /// is none.
    public let noticeJSON: String?

    public static let none = LegacyMigrationWire(noticeJSON: nil)

    init(noticeJSON: String?) {
        self.noticeJSON = noticeJSON
    }

    public init(from decoder: Decoder) throws {
        let value = try JSONValue(from: decoder)
        guard case .object(let fields) = value, case .object? = fields["notice"],
            let notice = fields["notice"]
        else {
            noticeJSON = nil
            return
        }
        let data = try JSONEncoder().encode(notice)
        noticeJSON = String(decoding: data, as: UTF8.self)
    }
}

/// The notice the Rust assembled: that the contributor's contributions now go
/// under their NEAR AI account, and whether their automatic folders were
/// kept. Every property is filled from the payload and none is written in
/// Swift.
public struct LegacyMigrationNotice: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    /// Whether automatic folders were kept, already chosen by the Rust.
    public let folders: String
    /// The button. It records that the notice was shown
    /// (`acknowledge_legacy_invite_migration`) and does nothing else.
    public let acknowledge: String

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = ["title", "body", "folders", "acknowledge"]

    /// Decode the payload, or nil if it will not parse or a field is empty:
    /// shown whole or not at all.
    public static func decode(fromJSON json: String) -> LegacyMigrationNotice? {
        guard let data = json.data(using: .utf8),
            let notice = try? JSONDecoder().decode(LegacyMigrationNotice.self, from: data)
        else {
            return nil
        }
        let sentences = [notice.title, notice.body, notice.folders, notice.acknowledge]
        return sentences.contains(where: \.isEmpty) ? nil : notice
    }
}
