//! Reading `GET /_ironwire/events`.
//
// The stream carries two kinds of line the app must not confuse. `control.rs`
// sends `: connected` the moment a client attaches, so a quiet system does not
// look like a hung one, and `: lagged N` when a subscriber fell far enough
// behind that the bus dropped events on it. Both are SSE *comments*, and a
// client that treats them as data will try to parse `connected` as JSON and
// either crash or spam a log every time somebody connects.
//
// The lag frame is also the reminder that this stream is not a history. The bus
// holds 256 events and drops rather than blocking the datapath (`events.rs`), so
// what arrives here is a hint that something changed. `/_ironwire/status` stays
// the source of truth.

import Foundation

/// What one line of an SSE stream turned out to be.
public enum SSELine: Sendable, Equatable {
    /// A comment. Includes `: connected` and `: lagged N`.
    case comment(String)
    /// A complete event, terminated by the blank line that followed it.
    case event(String)
    /// Nothing to hand on yet.
    case pending
}

/// Line-at-a-time SSE reader.
///
/// Fed from `URLSession.AsyncBytes.lines`, which already deals with chunk
/// boundaries and line endings — this handles the framing above it: comments,
/// multi-line `data:` fields, and the blank line that dispatches an event.
public struct SSEDecoder: Sendable {
    private var pending: [String] = []

    public init() {}

    public mutating func feed(_ line: String) -> SSELine {
        if line.hasPrefix(":") {
            return .comment(String(line.dropFirst()).trimmingCharacters(in: .whitespaces))
        }
        if line.isEmpty {
            guard !pending.isEmpty else { return .pending }
            let payload = pending.joined(separator: "\n")
            pending.removeAll()
            return .event(payload)
        }
        if line.hasPrefix("data:") {
            var value = Substring(line.dropFirst("data:".count))
            // One optional leading space belongs to the framing, not the value.
            if value.hasPrefix(" ") { value = value.dropFirst() }
            pending.append(String(value))
        }
        // Any other field (`event:`, `id:`, `retry:`) is not something this
        // stream sends and not something this app would do anything with.
        return .pending
    }
}
