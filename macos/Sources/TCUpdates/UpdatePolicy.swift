import Foundation

/// What this installation is allowed to do about updates.
public enum UpdateMode: Equatable, Sendable {
    /// We placed the bytes; Sparkle may run.
    case selfUpdating
    /// Homebrew placed the bytes. Sparkle must never be started, and the
    /// user is shown the command that does work.
    case managedByHomebrew(upgradeCommand: String)
    /// Neither. `reason` is a stable label, never a URL or a path, so it is
    /// safe to log and safe to show.
    case disabled(reason: String)
}

public enum UpdatePolicy {
    public static let noFeedReason = "update_feed_not_configured"
    public static let insecureFeedReason = "update_feed_not_https"

    /// The single decision point for whether Sparkle starts.
    ///
    /// Homebrew is checked first and unconditionally: there is never a case
    /// where this app and a package manager both believe they own the same
    /// file, and a Homebrew user seeing "updates unavailable" would be told
    /// something false when a working command exists.
    public static func mode(homebrew: HomebrewInstallState, feedURL: String?) -> UpdateMode {
        if homebrew.isManaged {
            return .managedByHomebrew(upgradeCommand: homebrew.upgradeCommand)
        }
        let trimmed = (feedURL ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return .disabled(reason: noFeedReason)
        }
        guard trimmed.lowercased().hasPrefix("https://") else {
            return .disabled(reason: insecureFeedReason)
        }
        return .selfUpdating
    }
}
