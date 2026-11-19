import Foundation

#if DEBUG
/// The throwaway defaults suite a screenshot capture draws its pane
/// switches from. `removePersistentDomain` clears the keys but leaves the
/// `<suite>.plist` file in `~/Library/Preferences`, so `run` deletes the
/// file too, and `sweepStale` clears what a killed run left behind.
enum CaptureSuite {
    static let prefix = "trace-commons-capture-"

    static var preferencesDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Preferences", isDirectory: true)
    }

    /// Runs `body` with a fresh suite and leaves no file behind.
    @discardableResult
    static func run<T>(_ body: (UserDefaults, String) -> T) -> T? {
        let name = prefix + UUID().uuidString
        guard let defaults = UserDefaults(suiteName: name) else { return nil }
        defer { remove(name, defaults) }
        return body(defaults, name)
    }

    /// `cfprefsd` writes the emptied domain back to disk some milliseconds
    /// after `removePersistentDomain`, so one delete is not enough: delete
    /// until the file has stayed gone across two checks.
    private static func remove(_ name: String, _ defaults: UserDefaults) {
        defaults.removePersistentDomain(forName: name)
        defaults.synchronize()
        let file = preferencesDirectory.appendingPathComponent(name + ".plist")
        var absentChecks = 0
        for _ in 0..<40 where absentChecks < 3 {
            if FileManager.default.fileExists(atPath: file.path) {
                try? FileManager.default.removeItem(at: file)
                absentChecks = 0
            } else {
                absentChecks += 1
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
    }

    /// Deletes capture-suite plists last written more than `age` seconds
    /// ago: a run killed before its cleanup. A live run's suite is newer.
    static func sweepStale(olderThan age: TimeInterval = 3600) {
        let fm = FileManager.default
        let cutoff = Date(timeIntervalSinceNow: -age)
        let urls = (try? fm.contentsOfDirectory(
            at: preferencesDirectory, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for url in urls where url.lastPathComponent.hasPrefix(prefix) && url.pathExtension == "plist" {
            let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if let modified, modified < cutoff { try? fm.removeItem(at: url) }
        }
    }
}
#endif
