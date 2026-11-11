import Foundation
import TCBridge
import TCShellCore

/// The launch check of staging design section 4.3: the bundle's Info.plist
/// `TCChannel` must be exactly the linked dylib's channel before anything
/// opens the state directory. The trace daemon and compute both pass
/// through here, so a staging bundle around a production dylib (or the
/// reverse, or a bundle naming no channel) starts neither.
enum BuildChannelGate {
    enum Verdict {
        case agreed(BuildChannel)
        /// Why this launch refuses: a hash-only `label` for logs and tests,
        /// and the `sentence` to show instead of starting.
        case refused(label: String, sentence: String)
    }

    /// The bundle's declared channel. Nil outside a bundle that declares
    /// one, which refuses.
    static var plistChannel: String? {
        Bundle.main.object(forInfoDictionaryKey: "TCChannel") as? String
    }

    /// The dylib's channel, read once: it is fixed for the process.
    static let dylibChannel: BuildChannel? = BuildChannel.decode(fromJSON: TCBuildChannel.json())

    /// The main window's persistent label (spec S7, 4.8): the core's word
    /// on a staging dylib, nil on production. Read once, like the channel.
    static let windowLabel: String? = TCBuildChannel.windowLabel()

    /// Whether the bundle's declared feature environment and profile digest
    /// are the linked dylib's. False when the dylib's identity is unreadable.
    static var featureProfileAgrees: Bool {
        guard let json = TCFeatureFlags.buildProfileJSON(),
              let profile = try? JSONDecoder().decode(BuildFeatureProfile.self, from: Data(json.utf8))
        else { return false }
        return profile.agrees(
            environment: Bundle.main.object(forInfoDictionaryKey: "TCFeatureEnvironment") as? String,
            digest: Bundle.main.object(forInfoDictionaryKey: "TCFeatureProfileDigest") as? String)
    }

    /// Both agreements, channel and feature profile, so every caller (and the
    /// `start(plistChannel:)` test seams) goes through the same check. The
    /// feature-profile result is a parameter only so a test can disagree
    /// without a bundle. Both refusals show the core's damaged-copy sentence:
    /// the bundle and the library disagree either way. The fallback is a
    /// label, not a sentence: the core's line is nil only on a caught panic.
    static func verdict(plistChannel: String?, featureProfileAgrees: Bool = BuildChannelGate.featureProfileAgrees) -> Verdict {
        guard featureProfileAgrees else {
            return .refused(label: "feature-profile-mismatch",
                            sentence: TCBuildChannel.mismatchLine() ?? "feature-profile-mismatch")
        }
        switch ChannelAgreement.verdict(plistChannel: plistChannel, dylib: dylibChannel) {
        case .agreed(let channel):
            return .agreed(channel)
        case .refused:
            return .refused(label: "channel-mismatch",
                            sentence: TCBuildChannel.mismatchLine() ?? "channel-mismatch")
        }
    }

    /// This launch's verdict.
    static var current: Verdict { verdict(plistChannel: plistChannel) }

    /// The agreed channel, or nil when this launch refused.
    static var agreedChannel: BuildChannel? {
        if case .agreed(let channel) = current { return channel }
        return nil
    }
}
