import Foundation

/// Whether Homebrew owns this installation, and what to tell the user if so.
public struct HomebrewInstallState: Equatable, Sendable {
    public let isManaged: Bool
    public let caskName: String
    /// The directory that proved it, or nil. Kept for diagnostics only; it
    /// is a fixed, non-secret path, but it is still not logged.
    public let caskroomPath: String?

    public init(isManaged: Bool, caskName: String, caskroomPath: String?) {
        self.isManaged = isManaged
        self.caskName = caskName
        self.caskroomPath = caskroomPath
    }

    /// The one command a Homebrew user should run. Shown verbatim in
    /// Settings so it can be copied without editing.
    public var upgradeCommand: String { "brew upgrade --cask \(caskName)" }
}

/// Detects a Homebrew cask installation by local path only.
///
/// No network, no `brew` subprocess. This runs on the launch path, and a
/// subprocess there is a hang waiting to happen; it would also make the
/// answer depend on the user's `PATH`, which is not where the truth lives.
/// The truth is whether a Caskroom directory for this cask exists.
public enum HomebrewDetector {
    /// Matches the cask name in the tap. Changing one without the other
    /// silently turns every Homebrew install back into a self-updating one.
    public static let caskName = "trace-commons"

    /// Apple silicon first, then the Intel prefix. Both are checked because
    /// a Rosetta or migrated install can sit under either.
    public static let defaultPrefixes = ["/opt/homebrew", "/usr/local"]

    public static func detect(
        prefixes: [String] = HomebrewDetector.defaultPrefixes,
        fileManager: FileManager = .default
    ) -> HomebrewInstallState {
        for prefix in prefixes {
            let path = (prefix as NSString)
                .appendingPathComponent("Caskroom")
            let candidate = (path as NSString).appendingPathComponent(caskName)
            var isDirectory: ObjCBool = false
            if fileManager.fileExists(atPath: candidate, isDirectory: &isDirectory),
                isDirectory.boolValue
            {
                return HomebrewInstallState(
                    isManaged: true, caskName: caskName, caskroomPath: candidate
                )
            }
        }
        return HomebrewInstallState(isManaged: false, caskName: caskName, caskroomPath: nil)
    }
}
