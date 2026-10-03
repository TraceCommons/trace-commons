import Foundation

/// The glass monitor's Traces words, decoded from
/// `tc_monitor_traces_copy_json` (`preview_copy::monitor_traces_copy`).
///
/// Every property comes from the payload: the inspector's row labels, the
/// review's actions, and what the tab says when the core does not answer.
/// They were written in Swift (`MonitorWords`); the core now holds them so
/// no shell writes its own.
///
/// Decoding is here rather than in `TCBridge` so it can be tested without
/// linking the dylib; `TCBridgeTests` checks it against the real export.
public struct MonitorTracesCopy: Decodable, Equatable, Sendable {
    public let review: String
    public let tool: String
    public let folder: String
    public let started: String
    public let length: String
    public let prompts: String
    public let size: String
    public let sends: String
    public let marks: String
    public let unsure: String
    public let eligibility: String
    public let attestation: String
    public let held: String
    public let sample: String
    public let residualRisk: String
    public let personalInformation: String
    public let secondLookWaiting: String
    public let contribute: String
    public let keep: String
    public let dismiss: String
    public let undoContribute: String
    public let undoKeep: String
    public let coreUnreachable: String
    public let requestFailed: String

    enum CodingKeys: String, CodingKey {
        case review, tool, folder, started, length, prompts, size, sends, marks, unsure
        case eligibility, attestation, held, sample
        case residualRisk = "residual_risk"
        case personalInformation = "personal_information"
        case secondLookWaiting = "second_look_waiting"
        case contribute, keep, dismiss
        case undoContribute = "undo_contribute"
        case undoKeep = "undo_keep"
        case coreUnreachable = "core_unreachable"
        case requestFailed = "request_failed"
    }

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = [
        "review", "tool", "folder", "started", "length", "prompts", "size", "sends", "marks", "unsure",
        "eligibility", "attestation", "held", "sample", "residual_risk", "personal_information",
        "second_look_waiting", "contribute", "keep", "dismiss", "undo_contribute", "undo_keep", "core_unreachable", "request_failed",
    ]

    /// Decode the payload, or nil if it will not parse or a field is empty.
    public static func decode(fromJSON json: String?) -> MonitorTracesCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(MonitorTracesCopy.self, from: data)
        else {
            return nil
        }
        let words = [
            copy.review, copy.tool, copy.folder, copy.started, copy.length, copy.prompts, copy.size,
            copy.sends, copy.marks, copy.unsure, copy.eligibility, copy.attestation, copy.held, copy.sample,
            copy.residualRisk, copy.personalInformation, copy.secondLookWaiting,
            copy.contribute, copy.keep, copy.dismiss,
            copy.undoContribute, copy.undoKeep, copy.coreUnreachable, copy.requestFailed,
        ]
        return words.contains(where: \.isEmpty) ? nil : copy
    }

    /// What the tab says for a failed request: the core's line for a core
    /// that does not answer, or for one that refused. Never the error's own
    /// fixed label, which is for logs.
    public func line(for error: DaemonDataError) -> String {
        if case .unreachable = error { return coreUnreachable }
        return requestFailed
    }
}
