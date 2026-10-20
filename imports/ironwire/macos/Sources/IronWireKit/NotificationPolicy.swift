//! Which events are worth interrupting someone for.
//
// This mirrors `Event::is_user_visible` in `crates/ironwire_proxy/src/events.rs`
// and deliberately holds no opinion of its own. The daemon already decides which
// events matter; a menu bar app that decided again, differently, is the drift
// this whole design exists to prevent.
//
// The predicate is narrow on purpose. Rungs 0–2 change nothing the user can
// observe, and announcing them trains people to ignore the channel — after which
// the one announcement that matters gets ignored too (`docs/DESIGN.md` §3). The
// bus is lossy by construction and this app is too.
//
// **Note on scope.** Issue #8 lists two notifiable events, `Routed` at
// `CrossFamily` and `Failed`. `Event::is_user_visible` returns true for a third,
// `CapReached`, with the reasoning that the user set that cap and then lost
// sight of it — and the issue's own problem statement names "a spend cap being
// reached" among the events that matter. Following the daemon's predicate keeps
// the two in step; it is published once per backend per window, so it cannot
// become the per-request noise the issue warns about. Narrowing it back is a
// one-case edit here.

import Foundation

public enum NotificationPolicy {
    /// The notification an event deserves, or `nil` for the events that deserve
    /// none.
    public static func notification(for event: Event) -> (title: String, body: String)? {
        switch event {
        // The one descent IronWire is obliged to announce: a different model
        // family is answering, with the prompt cache cold and reasoning state
        // dropped. The user can act on this; they cannot act on the rest.
        case .routed(_, _, let from, let to, let rung, _, let reason) where rung.isUserVisible:
            let where_ = from.map { "\($0) → \(to)" } ?? to
            return ("Different model family", "\(where_) — \(reason)")

        case .failed(_, _, let detail):
            return ("Request failed", detail)

        case .capReached(_, let backend, let spentUsd, let capUsd):
            return (
                "Spend cap reached",
                "\(backend): \(Format.currency(spentUsd)) of \(Format.currency(capUsd)) — not routing there until tomorrow"
            )

        // A circuit change is useful in a live view and noise as an alert: the
        // whole point of the breaker is that the request was still served.
        case .routed, .health, .unrecognised:
            return nil
        }
    }
}
