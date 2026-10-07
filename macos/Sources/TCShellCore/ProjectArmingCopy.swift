import Foundation

/// The words for arming a project -- setting it to contribute without
/// asking -- decoded from `tc_arming_offer_copy_json`
/// (`project_copy::arming_offer_copy`).
///
/// One table for both places arming is asked: the offer the queue shows once
/// a project has been contributed from several times (`evidence`, then
/// `question`), and the confirmation Settings shows before arming
/// (`question` as its heading, then `body`). It used to be written here and
/// in `ArmingOfferCopy`, transcribed from the Linux shell; the core now
/// holds the one copy every shell renders.
///
/// Arming is the strongest thing this window can be set to do, so neither
/// surface offers it without these words: a nil decode means no offer and no
/// confirmation, never a Swift-written one.
public struct ProjectArmingCopy: Decodable, Equatable, Sendable {
    /// "You've contributed from <project> N times." Stated before the
    /// question, so someone who reads only the first line learns why they are
    /// asked. Not rendered by a confirmation that has no count in hand.
    public let evidence: String
    /// "Contribute from <project> automatically?"
    public let question: String
    /// The button that arms. Carries the action.
    public let confirm: String
    /// The button that does not. Declining is a decision about this moment.
    public let decline: String
    /// The confirmation for arming from now: the scrubbing, that review
    /// stops, and the way back, in that order.
    public let body: String
    /// The confirmation for arming with the backlog (`include_backlog`).
    public let bodyWithBacklog: String
    /// Settings' confirmation before a mode is set to Automatic, in Ron's
    /// #1146 words: its heading, the line under it, and its two buttons.
    public let settingsQuestion: String
    public let settingsDescription: String
    public let settingsDecline: String
    public let settingsConfirm: String

    enum CodingKeys: String, CodingKey {
        case evidence, question, confirm, decline, body
        case bodyWithBacklog = "body_with_backlog"
        case settingsQuestion = "settings_question"
        case settingsDescription = "settings_description"
        case settingsDecline = "settings_decline"
        case settingsConfirm = "settings_confirm"
    }

    /// Decode the payload, or nil if it will not parse or a field is empty.
    public static func decode(fromJSON json: String?) -> ProjectArmingCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(ProjectArmingCopy.self, from: data)
        else {
            return nil
        }
        let sentences = [
            copy.evidence, copy.question, copy.confirm, copy.decline, copy.body,
            copy.bodyWithBacklog, copy.settingsQuestion, copy.settingsDescription,
            copy.settingsDecline, copy.settingsConfirm,
        ]
        return sentences.contains(where: \.isEmpty) ? nil : copy
    }
}
