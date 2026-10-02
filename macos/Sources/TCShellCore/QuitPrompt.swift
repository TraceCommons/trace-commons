import Foundation

/// The quit prompt that is true for this process, decoded from
/// `tc_quit_prompt_json` (`quit_copy::quit_prompt`).
///
/// Quitting must say what keeps running, and the true sentence depends on
/// whether this app hosts the watcher or is attached to one another process
/// runs. The ABI reads that off the daemon handle and chooses the body; this
/// shell shows it. It used to hard-code the hosting sentence, which is false
/// for an attached app.
public struct QuitPrompt: Decodable, Equatable, Sendable {
    /// `hosting`, `attached` or `unavailable`, as the ABI chose it.
    public let role: String
    public let title: String
    public let body: String
    public let confirm: String
    public let cancel: String

    /// Decode the payload, or nil if it will not parse or a field is empty.
    /// Nil means the quit cannot be confirmed: it is not confirmable before
    /// the true sentence for this process is shown.
    public static func decode(fromJSON json: String?) -> QuitPrompt? {
        guard let data = json?.data(using: .utf8),
            let prompt = try? JSONDecoder().decode(QuitPrompt.self, from: data)
        else {
            return nil
        }
        let sentences = [prompt.role, prompt.title, prompt.body, prompt.confirm, prompt.cancel]
        return sentences.contains(where: \.isEmpty) ? nil : prompt
    }
}
