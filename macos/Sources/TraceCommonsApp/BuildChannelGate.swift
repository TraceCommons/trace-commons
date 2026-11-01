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
        /// The sentence to show instead of starting.
        case refused(String)
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

    static func verdict(plistChannel: String?) -> Verdict {
        switch ChannelAgreement.verdict(plistChannel: plistChannel, dylib: dylibChannel) {
        case .agreed(let channel):
            return .agreed(channel)
        case .refused:
            // The fallback is a label, not a sentence: the core's line is
            // nil only on a caught panic.
            return .refused(TCBuildChannel.mismatchLine() ?? "channel-mismatch")
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
