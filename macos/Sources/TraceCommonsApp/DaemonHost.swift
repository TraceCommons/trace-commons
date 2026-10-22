import Foundation
import TCBridge
import TCShellCore

/// Where the in-process daemon keeps its state.
///
/// The resolution rules, and the tests that pin them, live in
/// `TCShellCore.StateDirectory` -- this app target links the FFI dylib, so
/// nothing in it can be a unit test, and the defect that made the shipped
/// build inert (resolving from an environment variable a Finder launch never
/// has) is exactly the kind one unit test catches.
///
/// The BOTH-not-either session-roots rule used to be transcribed here. It is
/// not any more. It lives once, in `daemon::settings::roots_declared`, and
/// the C ABI's start functions enforce it and report `roots-not-declared`;
/// see `TCDaemon.TCError.rootsNotDeclared`. A copy of a rule that decides
/// whether a developer's source tree gets scanned is a copy that can drift
/// from the one the daemon actually obeys.
enum DaemonHost {
    typealias Resolution = StateDirectory.Resolution
    typealias Refusal = StateDirectory.Refusal

    /// `channel` is the one `BuildChannelGate` agreed: the directory and its
    /// override variable are the dylib's.
    static func resolveConfigDirectory(channel: BuildChannel) throws -> Resolution {
        try StateDirectory.resolve(channel: channel, overrideCheck: overrideCheck)
    }

    /// The dylib's check of a directory taken from the override variable
    /// (`tc_resolve_state_dir`): the daemon and compute open their directory
    /// with `ConfigStore::open`, which does not run it (staging design 4.4).
    /// The sentence is the core's; a panic reading it leaves the label, so
    /// the refusal still shows something rather than nothing.
    static let overrideCheck = StateDirectory.OverrideCheck { dir in
        TCBuildChannel.overrideRefusal(dir).map { label in
            .init(label: label, sentence: TCBuildChannel.overrideRefusedLine() ?? label)
        }
    }
}
