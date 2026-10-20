//! Finding the daemon: where its home is, what its token is, what port it is on.
//
// Mirrors `PathsConfig::resolve` (`crates/ironwire_core/src/config.rs`) and the
// CLI's `control_token` (`src/commands/mod.rs`), with one difference that
// matters: the CLI *mints* a token when none exists, because it is often the
// thing setting the daemon up. This app never writes into `$IRONWIRE_HOME`. A
// GUI that creates a control token would be creating the credential for a
// daemon that may not exist, in a directory it does not own.

import Foundation

public enum Discovery {
    /// The port `ironwire serve` binds when nothing says otherwise
    /// (`ironwire_core::DEFAULT_PORT`).
    public static let defaultPort = 8463

    /// `$IRONWIRE_HOME`, or `~/.ironwire`.
    public static func home(environment: [String: String] = ProcessInfo.processInfo.environment) -> URL {
        if let explicit = environment["IRONWIRE_HOME"], !explicit.isEmpty {
            return URL(fileURLWithPath: (explicit as NSString).expandingTildeInPath)
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".ironwire")
    }

    /// The control token, or `nil` when there is none to read.
    ///
    /// `nil` is an ordinary state — it means the daemon has never run — and the
    /// caller renders "not running" for it rather than an error.
    public static func token(home: URL? = nil) -> String? {
        let path = (home ?? Discovery.home()).appendingPathComponent("control.token")
        guard let contents = try? String(contentsOf: path, encoding: .utf8) else { return nil }
        let trimmed = contents.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// The port from `config.toml`, or the default.
    ///
    /// Best-effort by design: a TOML parser is not worth a dependency for one
    /// integer, and getting this wrong costs a "not running" message rather than
    /// anything silent. `port` under any table other than `[server]` is ignored,
    /// which is the only mistake a naive search would actually make here.
    public static func port(home: URL? = nil) -> Int {
        let path = (home ?? Discovery.home()).appendingPathComponent("config.toml")
        guard let contents = try? String(contentsOf: path, encoding: .utf8) else { return defaultPort }
        return port(inTOML: contents)
    }

    /// Split out so the rule above is testable without a filesystem.
    public static func port(inTOML contents: String) -> Int {
        var section = ""
        for rawLine in contents.split(separator: "\n", omittingEmptySubsequences: false) {
            var line = rawLine.trimmingCharacters(in: .whitespaces)
            if let comment = line.firstIndex(of: "#") { line = String(line[..<comment]).trimmingCharacters(in: .whitespaces) }
            if line.hasPrefix("[") , line.hasSuffix("]") {
                section = String(line.dropFirst().dropLast()).trimmingCharacters(in: .whitespaces)
                continue
            }
            guard section == "server" else { continue }
            let parts = line.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard parts.count == 2,
                  parts[0].trimmingCharacters(in: .whitespaces) == "port",
                  let value = Int(parts[1].trimmingCharacters(in: .whitespaces)),
                  (1...65535).contains(value)
            else { continue }
            return value
        }
        return defaultPort
    }

    /// The address every control call is made against.
    ///
    /// One construction, used by `ControlClient` and by the menu item that puts
    /// it on the clipboard — a second copy of this string is how the URL a user
    /// is handed stops being the URL the app is talking to.
    ///
    /// Loopback only, and no token: this returns somewhere to point `curl`, and
    /// the credential that would make it answer is the user's to fetch.
    public static func controlURL(port: Int, path: String = "") -> URL {
        var components = URLComponents()
        components.scheme = "http"
        components.host = "127.0.0.1"
        components.port = port
        components.path = "/_ironwire" + path
        // A literal host with an integer port cannot fail to compose. The
        // fallback is here so the signature can promise a URL rather than make
        // every caller handle an impossibility.
        return components.url ?? URL(fileURLWithPath: "/")
    }

    /// Where an installed `ironwire` binary is likely to be.
    ///
    /// A GUI app does not inherit the user's shell `PATH`, so `which` is not
    /// available to us. This is a best-effort list of the locations the five
    /// install channels actually use (`docs/PACKAGING.md`); finding nothing is a
    /// normal outcome and the menu says so rather than guessing.
    public static func daemonBinary(fileManager: FileManager = .default,
                                    home: URL? = nil) -> URL? {
        let candidates = [
            (home ?? Discovery.home()).appendingPathComponent("bin/ironwire"),
            URL(fileURLWithPath: "/opt/homebrew/bin/ironwire"),
            URL(fileURLWithPath: "/usr/local/bin/ironwire"),
            URL(fileURLWithPath: "/usr/bin/ironwire"),
        ]
        return candidates.first { fileManager.isExecutableFile(atPath: $0.path) }
    }

    /// The log `brew services` writes to, when this is a Homebrew install.
    ///
    /// Only offered when it exists: a menu item that opens nothing is worse than
    /// one that is not there. A foreground `ironwire serve` logs to its own
    /// terminal and there is no file to point at.
    public static func brewLog(fileManager: FileManager = .default) -> URL? {
        let candidates = [
            URL(fileURLWithPath: "/opt/homebrew/var/log/ironwire.log"),
            URL(fileURLWithPath: "/usr/local/var/log/ironwire.log"),
        ]
        return candidates.first { fileManager.fileExists(atPath: $0.path) }
    }
}
