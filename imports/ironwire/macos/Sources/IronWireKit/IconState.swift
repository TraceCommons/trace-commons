//! What the menu bar icon says without being read.
//
// The icon is the reason this app exists. `ironwire status` already tells a user
// everything, if they think to ask; the icon is the part that reaches someone
// who is not asking, which is the situation every event worth announcing
// happens in (`crates/ironwire_proxy/src/events.rs`).
//
// So the derivation has to be honest and it has to be cheap to read. Every input
// below is a value the daemon computed and published. Nothing here works
// anything out from a backend's *name*.

import Foundation

/// What the icon is currently saying.
public enum IconState: Sendable, Equatable {
    /// On a preferred route, nothing skipped, somewhere to go.
    case healthy
    /// The last route sat below `preferred`, but not far enough to interrupt
    /// anyone over.
    case degraded
    /// A cross-family descent, a backend being skipped, or nowhere left to
    /// route. The states worth looking up for.
    case attention
    /// No daemon answered.
    case unreachable

    /// Read the icon out of a status document.
    ///
    /// Ordered most severe first. The single judgement call is that an
    /// unrecognised rung counts as degraded rather than fine — see
    /// `Rung.isDegraded` for why that direction is the safe one.
    public static func from(_ status: StatusView?) -> IconState {
        guard let status else { return .unreachable }

        // A backend the breaker is skipping looks identical to an idle one
        // otherwise, which is exactly the confusion the circuit state exists to
        // clear up.
        if status.backends.contains(where: { $0.health.isOpen }) { return .attention }

        // Nowhere to go. `unknown` pools count as somewhere, because the daemon
        // will still try them — "we have not heard from it" is not "it is down"
        // (`BalanceView` in `control.rs` keeps the two apart for the same
        // reason).
        if status.balance.available == 0 && status.balance.unknown == 0 { return .attention }

        if let rung = status.lastRoute?.rung {
            if rung.isUserVisible { return .attention }
            if rung.isDegraded { return .degraded }
        }
        return .healthy
    }

    /// One line for the top of the menu, so the icon's meaning is never a guess.
    public var summary: String {
        switch self {
        case .healthy: return "Routing normally"
        case .degraded: return "Running below the preferred route"
        case .attention: return "Needs a look"
        case .unreachable: return "IronWire is not running"
        }
    }
}
