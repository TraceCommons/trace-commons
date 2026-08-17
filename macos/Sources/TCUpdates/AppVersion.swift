import Foundation

/// A three-part numeric version: `X.Y.Z`, all components non-negative
/// integers.
///
/// Deliberately not semver. `release-apps.yml` refuses to cut a tag that is
/// not `X.Y.Z`, so pre-release and build-metadata suffixes cannot appear in
/// anything this app is offered. Accepting a shape the pipeline cannot
/// produce would only widen what a hostile appcast can say.
public struct AppVersion: Comparable, Sendable, CustomStringConvertible {
    public let major: Int
    public let minor: Int
    public let patch: Int

    public init(major: Int, minor: Int, patch: Int) {
        self.major = major
        self.minor = minor
        self.patch = patch
    }

    /// Parses `X.Y.Z`, or returns nil. No trimming, no coercion, no
    /// leading `v`: a version this parser had to guess at is a version this
    /// app should not act on.
    public init?(_ string: String) {
        let parts = string.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 3 else { return nil }
        var values: [Int] = []
        for part in parts {
            guard !part.isEmpty,
                part.allSatisfy({ $0.isASCII && $0.isNumber }),
                let value = Int(part)
            else { return nil }
            values.append(value)
        }
        self.major = values[0]
        self.minor = values[1]
        self.patch = values[2]
    }

    public var description: String { "\(major).\(minor).\(patch)" }

    public static func < (lhs: AppVersion, rhs: AppVersion) -> Bool {
        (lhs.major, lhs.minor, lhs.patch) < (rhs.major, rhs.minor, rhs.patch)
    }
}

public enum VersionError: Error, Equatable {
    /// Not a three-part numeric version. The payload names which side was
    /// bad, and carries no version string, so this stays safe to log.
    case malformed(String)
}

public enum VersionComparison {
    /// True when `offered` is strictly greater than `current`.
    ///
    /// Strictly: equal is false, so a replayed appcast for the running
    /// version installs nothing; older is false, so a signed-but-old appcast
    /// replayed at a client cannot walk it backwards onto a build with known
    /// problems. Downgrade protection is this comparison and nothing else.
    public static func isNewer(current: String, offered: String) throws -> Bool {
        guard let currentVersion = AppVersion(current) else {
            throw VersionError.malformed("current")
        }
        guard let offeredVersion = AppVersion(offered) else {
            throw VersionError.malformed("offered")
        }
        return offeredVersion > currentVersion
    }
}
