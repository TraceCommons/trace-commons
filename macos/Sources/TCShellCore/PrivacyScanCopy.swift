import Foundation

/// The extra privacy scan's words, decoded from `tc_privacy_scan_copy_json`
/// (`privacy_scan_copy::privacy_scan_copy`).
///
/// The disclosure has two halves and both must stay: message text really
/// does leave this machine to a third party before Trace Commons sees it,
/// and if that scanner is unreachable nothing is sent at all. The core holds
/// both in one sentence so no shell can cut either; this shell renders it.
public struct PrivacyScanCopy: Decodable, Equatable, Sendable {
    public let title: String
    /// Local scrubbing always runs; it is not the thing being chosen.
    public let localAlways: String
    /// What the second scanner is and what it reads.
    public let offer: String
    /// Both halves of the disclosure.
    public let disclosure: String
    /// The choice that keeps message text on this machine.
    public let localOnly: String
    /// The choice that adds the second scanner.
    public let withNear: String
    /// The recovery prompt's heading, while the daemon holds uploads for
    /// `near-ai-notice-not-acknowledged`.
    public let recoveryTitle: String
    /// Why uploads are held, and what confirming does.
    public let recoveryDetail: String
    /// The button that opens the notice.
    public let recoveryAction: String

    enum CodingKeys: String, CodingKey {
        case title, offer, disclosure
        case localAlways = "local_always"
        case localOnly = "local_only"
        case withNear = "with_near"
        case recoveryTitle = "recovery_title"
        case recoveryDetail = "recovery_detail"
        case recoveryAction = "recovery_action"
    }

    /// Decode the payload, or nil if it will not parse or a field is empty.
    public static func decode(fromJSON json: String?) -> PrivacyScanCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(PrivacyScanCopy.self, from: data)
        else {
            return nil
        }
        let sentences = [
            copy.title, copy.localAlways, copy.offer, copy.disclosure, copy.localOnly,
            copy.withNear, copy.recoveryTitle, copy.recoveryDetail, copy.recoveryAction,
        ]
        return sentences.contains(where: \.isEmpty) ? nil : copy
    }
}
