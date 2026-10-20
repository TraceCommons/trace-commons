//! Starting with the machine.
//
// A menu bar app that reports on a background daemon is only useful if it is
// running, and asking someone to launch it by hand every morning is asking them
// to stop using it. `SMAppService` is the supported way to register a login item
// without a helper bundle or a `LaunchAgent` plist we would have to write and
// keep in step.
//
// Everything here reports what actually happened rather than what was asked for.
// A login item can be registered and then switched off by the user in System
// Settings, and a toggle that stays on while the system says otherwise is a
// toggle that lies.

import ServiceManagement
import SwiftUI

@MainActor
final class LoginItem: ObservableObject {
    /// What the system says about this app's login item.
    enum State: Equatable {
        /// Registered and allowed to run.
        case enabled
        /// Not registered.
        case disabled
        /// Registered, but the user has to allow it in System Settings — which
        /// macOS asks about the first time, and which we cannot answer for them.
        case awaitingApproval
        /// The attempt failed, with what the system said.
        case failed(String)
    }

    @Published private(set) var state: State = .disabled

    init() {
        refresh()
    }

    /// Ask the system, rather than remembering what we last asked for.
    func refresh() {
        switch SMAppService.mainApp.status {
        case .enabled: state = .enabled
        case .requiresApproval: state = .awaitingApproval
        case .notRegistered, .notFound: state = .disabled
        @unknown default: state = .disabled
        }
    }

    /// Whether the toggle should read as on.
    ///
    /// `awaitingApproval` counts as on: the registration exists, and what is
    /// missing is the user's answer to a system prompt, not another click here.
    var isOn: Bool {
        switch state {
        case .enabled, .awaitingApproval: return true
        case .disabled, .failed: return false
        }
    }

    /// What to say under the toggle, when there is anything worth saying.
    var detail: String? {
        switch state {
        case .enabled, .disabled:
            return nil
        case .awaitingApproval:
            return "Allow it in System Settings > General > Login Items."
        case .failed(let message):
            // Most often: the app is being run from a build directory rather
            // than installed. Say so, because it is fixable.
            return "Could not set this: \(message)"
        }
    }

    func set(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            refresh()
        } catch {
            state = .failed(error.localizedDescription)
        }
    }
}
