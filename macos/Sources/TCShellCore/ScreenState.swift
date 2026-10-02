import Foundation

/// The one state-precedence rule every glass screen draws from (#1173,
/// stack-wide review): what a screen shows when its signals disagree.
///
/// Precedence, highest first:
/// 1. `coreDown`: the core is not answering. Whatever was last read stays
///    on screen under the core's line, never drawn as current.
/// 2. `loading`: nothing has been read yet. Never drawn as empty or zero.
/// 3. `paused`: watching is paused. Drawn as paused, never as healthy.
/// 4. `unknown`: the core answered without the signal this screen needs.
///    Drawn as unknown (a dash), never as healthy and never as off.
/// 5. `ready`: everything the screen needs was reported.
///
/// A missing signal is never drawn as healthy: every input that is nil
/// lands above `ready`.
public enum ScreenState: Equatable, Sendable {
    case coreDown
    case loading
    case paused
    case unknown
    case ready

    /// - Parameters:
    ///   - failure: the screen's last failed read, if any.
    ///   - loaded: whether the screen has read anything yet.
    ///   - paused: watching paused, or nil when the core did not say.
    ///   - known: whether the core reported every signal the screen needs.
    public static func resolve(
        failure: DaemonDataError?, loaded: Bool, paused: Bool?, known: Bool
    ) -> ScreenState {
        if case .unreachable = failure { return .coreDown }
        if !loaded { return .loading }
        if paused == true { return .paused }
        if paused == nil || !known { return .unknown }
        return .ready
    }

    /// Whether the screen may draw its signals as current and healthy.
    public var isHealthy: Bool { self == .ready }
}
