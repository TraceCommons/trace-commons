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
/// The truth is whether a Caskroom directory for this cask exists AND the
/// running bundle is the app that cask installed. A Caskroom entry alone says
/// only that the cask is installed somewhere on the machine; a staging build,
/// a dev build or a DMG copy running beside it is not Homebrew's to update.
public enum HomebrewDetector {
    /// The production channel's bundle identifier (macos/channels/production.env).
    /// The cask installs only the production app, so no other identifier,
    /// the staging one included, can be Homebrew-managed.
    public static let productionBundleIdentifier = "ai.tracecommons.shell"

    /// Where Homebrew puts cask apps unless the user passed `--appdir`.
    public static let defaultAppDirectories = ["/Applications"]

    /// The running app, as far as this check needs to know it.
    public struct RunningBundle: Equatable, Sendable {
        public let identifier: String?
        public let path: String
        public let appDirectories: [String]

        public init(
            identifier: String?, path: String,
            appDirectories: [String] = HomebrewDetector.defaultAppDirectories
        ) {
            self.identifier = identifier
            self.path = path
            self.appDirectories = appDirectories
        }

        public static func current(_ bundle: Bundle = .main) -> RunningBundle {
            RunningBundle(identifier: bundle.bundleIdentifier, path: bundle.bundlePath)
        }
    }

    /// True when `running` is the production app sitting at the cask's
    /// target: `<appdir>/<its own name>.app`, symlinks resolved.
    static func isCaskCopy(_ running: RunningBundle) -> Bool {
        guard running.identifier == productionBundleIdentifier else { return false }
        let bundleURL = URL(fileURLWithPath: running.path)
        let name = bundleURL.lastPathComponent
        // Resolve the parent directories, which exist; the leaf may be a
        // path that does not, and symlink resolution is unreliable on those.
        let bundleParent = realPath(bundleURL.deletingLastPathComponent().path)
        return running.appDirectories.contains { realPath($0) == bundleParent }
            && !name.isEmpty
    }

    private static func realPath(_ path: String) -> String {
        guard let resolved = realpath(path, nil) else { return path }
        defer { free(resolved) }
        return String(cString: resolved)
    }

    /// Matches the cask name in the tap. Changing one without the other
    /// silently turns every Homebrew install back into a self-updating one.
    public static let caskName = "trace-commons"

    /// Apple silicon first, then the Intel prefix. Both are checked because
    /// a Rosetta or migrated install can sit under either.
    public static let defaultPrefixes = ["/opt/homebrew", "/usr/local"]

    public static func detect(
        prefixes: [String] = HomebrewDetector.defaultPrefixes,
        running: RunningBundle = .current(),
        fileManager: FileManager = .default
    ) -> HomebrewInstallState {
        guard isCaskCopy(running) else {
            return HomebrewInstallState(isManaged: false, caskName: caskName, caskroomPath: nil)
        }
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
