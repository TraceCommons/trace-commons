import Foundation
import ServiceManagement

/// Registers Trace Commons as a login item via `SMAppService`, per the
/// design spec (`docs/superpowers/specs/2026-08-08-contributor-shell-macos-design.md`,
/// "## Login item").
///
/// This is the ONLY macOS mechanism used for "run at login" -- the app never
/// writes a LaunchAgent plist itself. `daemon install` (the Linux path) stays
/// out of this binary entirely. `SMAppService` is what makes the app show up
/// in System Settings -> General -> Login Items, which is where a
/// contributor goes to audit background software; anything that runs at
/// login without appearing there reads as malware, correctly.
@MainActor
enum LoginItemManager {
    /// Mirrors `SMAppService.Status` with names a view can switch on without
    /// importing `ServiceManagement` itself.
    enum State: Equatable {
        /// Never registered, or registration was later undone.
        case notRegistered
        /// Registered and will run at login.
        case enabled
        /// Registered, but the user has to approve it in System Settings ->
        /// General -> Login Items before it will actually run. This is NOT
        /// an error state -- it is the expected result of `register()` on a
        /// system where the user (or a prior denial) has not yet approved
        /// this app, and it must be told apart from a real failure so the UI
        /// can point at System Settings instead of retrying or complaining.
        case requiresApproval
        /// The service was registered but has since been disabled outside
        /// this app (e.g. the user turned it off in System Settings).
        case notFound

        fileprivate init(_ status: SMAppService.Status) {
            switch status {
            case .notRegistered: self = .notRegistered
            case .enabled: self = .enabled
            case .requiresApproval: self = .requiresApproval
            case .notFound: self = .notFound
            @unknown default: self = .notRegistered
            }
        }
    }

    /// The live registration state, read fresh from `SMAppService` every
    /// call rather than cached -- the user can flip this from outside the
    /// app (System Settings) at any time, and a cached bool would then lie.
    static var currentState: State {
        State(SMAppService.mainApp.status)
    }

    /// Whether the app currently runs at login. `.requiresApproval` reports
    /// `false` here on purpose: the service is registered but will not
    /// actually launch until the user approves it, so "is it running at
    /// login" is honestly no for that state too.
    static var isEnabled: Bool {
        currentState == .enabled
    }

    enum RegisterOutcome: Equatable {
        case enabled
        case requiresApproval
        case failed(String)
    }

    /// Registers the app as a login item. Returns the resulting state rather
    /// than throwing past the caller -- `OnboardingDoneView` and
    /// `SettingsView` both need to render `.requiresApproval` as guidance,
    /// not as an error banner.
    @discardableResult
    static func register() -> RegisterOutcome {
        do {
            try SMAppService.mainApp.register()
        } catch {
            return .failed(error.localizedDescription)
        }
        switch currentState {
        case .enabled: return .enabled
        case .requiresApproval: return .requiresApproval
        case .notRegistered, .notFound: return .requiresApproval
        }
    }

    enum UnregisterOutcome: Equatable {
        case notRegistered
        case failed(String)
    }

    @discardableResult
    static func unregister() -> UnregisterOutcome {
        do {
            try SMAppService.mainApp.unregister()
        } catch {
            return .failed(error.localizedDescription)
        }
        return .notRegistered
    }
}
