import Foundation

/// The quit prompt, decoded from `tc_quit_reason_prompt_json`
/// (`quit_copy::quit_prompt`), and when to show it.
///
/// Quitting asks only when it would stop something live: a running compute
/// worker, or Private AI answering model calls. The prompt's body is one
/// short sentence naming which (owner, 2026-11-12).
public struct QuitPrompt: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    public let confirm: String
    public let cancel: String

    /// Decode the payload, or nil if it will not parse or a field is empty.
    public static func decode(fromJSON json: String?) -> QuitPrompt? {
        guard let data = json?.data(using: .utf8),
            let prompt = try? JSONDecoder().decode(QuitPrompt.self, from: data)
        else {
            return nil
        }
        let sentences = [prompt.title, prompt.body, prompt.confirm, prompt.cancel]
        return sentences.contains(where: \.isEmpty) ? nil : prompt
    }

    public enum Decision: Equatable, Sendable {
        /// Nothing live: quit without asking.
        case quitNow
        /// Show this prompt and quit only if it is confirmed.
        case ask(QuitPrompt)
        /// Something is live but the prompt could not be read: do not quit.
        case refuse
    }

    /// Whether quitting would stop compute: a snapshot exists and does not
    /// say its worker stopped. No snapshot means compute never opened, so
    /// nothing runs; a snapshot without the field counts as running, since
    /// asking once too often beats a silent stop.
    public static func stopsCompute(_ snapshot: ComputeSnapshot?) -> Bool {
        guard let snapshot else { return false }
        return snapshot.workerStopped != true
    }

    /// What a quit request does. `promptJSON` is asked only when something
    /// is live, so a failed read refuses the quit rather than becoming a
    /// silent one.
    public static func decision(
        stopsCompute: Bool,
        stopsPrivateAI: Bool,
        promptJSON: (Bool, Bool) -> String?
    ) -> Decision {
        guard stopsCompute || stopsPrivateAI else { return .quitNow }
        guard let prompt = decode(fromJSON: promptJSON(stopsCompute, stopsPrivateAI)) else {
            return .refuse
        }
        return .ask(prompt)
    }
}
