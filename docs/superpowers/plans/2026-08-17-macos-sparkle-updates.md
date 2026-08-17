# macOS Sparkle Auto-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The installed macOS contributor app discovers, verifies, and installs new releases through Sparkle 2.x — unless Homebrew installed it, in which case the app never touches its own bytes and says so.

**Architecture:** All the logic that can be tested without a framework lives in a new dependency-free SwiftPM library target, `TCUpdates`: three-part version comparison, Homebrew Caskroom detection, and the policy that turns those two facts into an `UpdateMode`. The app target keeps a thin `UpdateController` that owns the `SPUStandardUpdaterController` and starts it *only* when the policy says `.selfUpdating`. Sparkle arrives as a binary XCFramework through SwiftPM, so `make-app-bundle.sh` copies the `macos-arm64_x86_64` framework slice into `Contents/Frameworks` and signs it inside-out; `make-release-dmg.sh` repeats that signing with the Developer ID identity before notarization.

**Tech Stack:** Swift 6 / SwiftPM (`swift-tools-version:6.0`), SwiftUI, Sparkle 2.9.6 (binary XCFramework via SwiftPM), bash, `codesign`, `ditto`, `notarytool`, and the existing Rust integration test `crates/trace-commons-contributor/tests/release_pipeline.rs` for asserting the shell scripts' contract.

## Global Constraints

- Sparkle is a deliberate, recorded waiver of the repo's dependency policy (see `docs/superpowers/specs/2026-08-17-desktop-auto-update-design.md`, "Recorded deviation"). **Do NOT add any other new dependency** — not to `macos/Package.swift`, not to any `Cargo.toml`.
- Sparkle version is pinned **exactly**: `.package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.9.6")`. Commit `macos/Package.resolved`.
- The app targets macOS 14 (`platforms: [.macOS(.v14)]`, `LSMinimumSystemVersion` `14.0`). Sparkle 2.x requires macOS 12+. Do not lower the app's floor.
- Swift tools version stays `6.0`. `TCBridge` and every new target use `.swiftLanguageMode(.v6)`. `TraceCommonsApp` and `tc-ffi-demo` keep `.swiftLanguageMode(.v5)` — do not change them.
- The release job builds on `macos-26` (Swift 6.3). Do not lower the runner image: `swift build --arch arm64 --arch x86_64` fails with "duplicate output file" on `macos-15`.
- The DMG must remain **universal** (`swift build --arch arm64 --arch x86_64`), Developer ID signed, notarized and stapled. An unsigned or un-notarized artifact named like a release is worse than no artifact.
- Fail closed. If the Sparkle public EdDSA key is not supplied at bundle-assembly time, the bundle ships **no** `SUFeedURL` and `SUEnableAutomaticChecks` set to `<false/>`, and `make-release-dmg.sh` refuses to run at all. There is no unverified update path.
- Never sign with `codesign --deep`. Sign inner code first, outer bundle last. `--deep` breaks the Downloader XPC service's entitlements and is the single most common Sparkle signing failure. (`codesign --verify --deep` is fine — verification is not signing.)
- Info.plist values, exactly: `SUFeedURL` = `https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml`, `SUPublicEDKey` = the base64 key from `TC_SPARKLE_PUBLIC_ED_KEY`, `SUEnableAutomaticChecks` = `<true/>`, `SUAutomaticallyUpdate` = `<false/>`, `SUScheduledCheckInterval` = `86400`.
- Homebrew cask name is `trace-commons`; the command shown to users is exactly `brew upgrade --cask trace-commons`. Homebrew prefixes checked: `/opt/homebrew` and `/usr/local`.
- Hash-only / label-only logging. Never log the feed URL, the public key, signatures, or download bodies.
- No emojis in code, comments, commits, or PR bodies. Short imperative commit subjects, no `feat:`/`fix:` prefixes.

### Prerequisite before any `swift build` or `swift test` in this plan

`macos/Package.swift` links the Rust FFI dylib with `-ltrace_commons_contributor_ffi`, and `swift test` builds every target in the package, including the executables. From the repo root, run this once per session before any Swift command below:

```bash
cargo build -p trace-commons-contributor-ffi
```

Without it, every `swift test` invocation fails at link time with `library 'trace_commons_contributor_ffi' not found`, which looks like a bug in this plan's code and is not.

---

### Task 1: A dependency-free updates library with version comparison

**Files:**
- Modify: `macos/Package.swift:24-56` (add two targets to the `targets:` array)
- Create: `macos/Sources/TCUpdates/AppVersion.swift`
- Create: `macos/Tests/TCUpdatesTests/AppVersionTests.swift`
- Create: `tests/fixtures/update-conformance/version-comparison.json`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `public struct AppVersion: Comparable, Sendable, CustomStringConvertible` with `public let major: Int`, `public let minor: Int`, `public let patch: Int`, `public init?(_ string: String)`, `public var description: String`.
  - `public enum VersionError: Error, Equatable { case malformed(String) }`
  - `public enum VersionComparison { public static func isNewer(current: String, offered: String) throws -> Bool }`

This mirrors the Rust `update::version::is_newer(current, offered) -> Result<bool, VersionError>` from `docs/superpowers/plans/2026-08-17-update-manifest-publishing.md` Task 2. The two implementations read the **same** fixture file, which is the only thing that keeps them from drifting apart.

- [ ] **Step 1: Create the shared conformance fixture**

Create `tests/fixtures/update-conformance/version-comparison.json`:

```json
{
  "schema_version": "trace_commons.update_conformance.version.v1",
  "comment": "Shared by the Swift updater (macos/Tests/TCUpdatesTests) and the Rust updater (crates/trace-commons-contributor/src/update/version.rs). A case removed here is a check silently dropped from both implementations at once; add, do not delete.",
  "comparisons": [
    { "current": "0.1.0", "offered": "0.1.1", "newer": true },
    { "current": "0.1.9", "offered": "0.2.0", "newer": true },
    { "current": "0.9.9", "offered": "1.0.0", "newer": true },
    { "current": "0.9.0", "offered": "0.10.0", "newer": true },
    { "current": "0.10.0", "offered": "0.9.0", "newer": false },
    { "current": "1.2.3", "offered": "1.2.3", "newer": false },
    { "current": "1.2.3", "offered": "1.2.2", "newer": false },
    { "current": "2.0.0", "offered": "1.9.9", "newer": false },
    { "current": "1.0.0", "offered": "1.0.10", "newer": true },
    { "current": "10.0.0", "offered": "9.99.99", "newer": false }
  ],
  "malformed": [
    { "current": "1.2", "offered": "1.2.3" },
    { "current": "1.2.3", "offered": "1.2.3.4" },
    { "current": "1.2.3", "offered": "v1.2.4" },
    { "current": "1.2.3", "offered": "1.2.x" },
    { "current": "", "offered": "1.2.3" },
    { "current": "1.2.3", "offered": "" },
    { "current": "1.2.3", "offered": "1.-2.3" },
    { "current": "1.2.3", "offered": " 1.2.4" }
  ]
}
```

If this file already exists because the CLI/manifest plan landed first, keep the existing file and append any of the above cases it is missing rather than overwriting it.

- [ ] **Step 2: Add the library and test targets to Package.swift**

In `macos/Package.swift`, insert these two entries into the `targets:` array immediately after the `.target(name: "TCBridge", ...)` entry (currently ending at line 33):

```swift
        // Update logic that must be testable without a framework, a bundle,
        // or a running app. Deliberately depends on nothing: the moment this
        // target imports Sparkle, `swift test` needs Sparkle.framework
        // present at runtime and the unit tests stop being unit tests.
        .target(
            name: "TCUpdates",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "TCUpdatesTests",
            dependencies: ["TCUpdates"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
```

- [ ] **Step 3: Write the failing test**

Create `macos/Tests/TCUpdatesTests/AppVersionTests.swift`:

```swift
import Foundation
import XCTest

@testable import TCUpdates

/// Locates `tests/fixtures/update-conformance/` from this source file's own
/// path. The fixtures live outside the SwiftPM package directory because the
/// Rust updater reads the same files; copying them into a resource bundle
/// would produce two files that drift.
enum ConformanceFixtures {
    static var directory: URL {
        // .../macos/Tests/TCUpdatesTests/AppVersionTests.swift
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // TCUpdatesTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // macos
            .deletingLastPathComponent()   // <repo root>
            .appendingPathComponent("tests/fixtures/update-conformance")
    }

    static func json(_ name: String) throws -> [String: Any] {
        let url = directory.appendingPathComponent(name)
        let data = try Data(contentsOf: url)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(
                domain: "ConformanceFixtures", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "\(name) is not a JSON object"]
            )
        }
        return object
    }
}

final class AppVersionTests: XCTestCase {
    func testFixtureComparisonsAllHold() throws {
        let fixture = try ConformanceFixtures.json("version-comparison.json")
        let cases = try XCTUnwrap(fixture["comparisons"] as? [[String: Any]])
        XCTAssertGreaterThanOrEqual(cases.count, 10, "the fixture lost cases")

        for entry in cases {
            let current = try XCTUnwrap(entry["current"] as? String)
            let offered = try XCTUnwrap(entry["offered"] as? String)
            let expected = try XCTUnwrap(entry["newer"] as? Bool)
            let actual = try VersionComparison.isNewer(current: current, offered: offered)
            XCTAssertEqual(
                actual, expected,
                "isNewer(current: \(current), offered: \(offered)) should be \(expected)"
            )
        }
    }

    func testFixtureMalformedVersionsAreRefusedNotGuessed() throws {
        let fixture = try ConformanceFixtures.json("version-comparison.json")
        let cases = try XCTUnwrap(fixture["malformed"] as? [[String: Any]])
        XCTAssertGreaterThanOrEqual(cases.count, 8, "the fixture lost cases")

        for entry in cases {
            let current = try XCTUnwrap(entry["current"] as? String)
            let offered = try XCTUnwrap(entry["offered"] as? String)
            XCTAssertThrowsError(
                try VersionComparison.isNewer(current: current, offered: offered),
                "isNewer(current: \(current), offered: \(offered)) must throw, not guess"
            ) { error in
                XCTAssertTrue(error is VersionError, "wrong error type: \(error)")
            }
        }
    }

    func testEqualVersionIsNotNewerSoAReplayedManifestInstallsNothing() throws {
        XCTAssertFalse(try VersionComparison.isNewer(current: "1.4.2", offered: "1.4.2"))
    }

    func testOlderVersionIsNotNewerSoAReplayedOldManifestCannotDowngrade() throws {
        XCTAssertFalse(try VersionComparison.isNewer(current: "1.4.2", offered: "1.4.1"))
    }

    func testAppVersionParsesAndRoundTrips() throws {
        let version = try XCTUnwrap(AppVersion("12.34.56"))
        XCTAssertEqual(version.major, 12)
        XCTAssertEqual(version.minor, 34)
        XCTAssertEqual(version.patch, 56)
        XCTAssertEqual(version.description, "12.34.56")
    }

    func testAppVersionRejectsNonNumericComponents() {
        XCTAssertNil(AppVersion("1.2.beta"))
        XCTAssertNil(AppVersion("1.2"))
        XCTAssertNil(AppVersion("1.2.3.4"))
        XCTAssertNil(AppVersion("v1.2.3"))
        XCTAssertNil(AppVersion(""))
    }

    func testAppVersionOrders() throws {
        let older = try XCTUnwrap(AppVersion("1.9.0"))
        let newer = try XCTUnwrap(AppVersion("1.10.0"))
        XCTAssertLessThan(older, newer)
        XCTAssertEqual(try XCTUnwrap(AppVersion("1.2.3")), try XCTUnwrap(AppVersion("1.2.3")))
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

```bash
cargo build -p trace-commons-contributor-ffi
swift test --package-path macos --filter AppVersionTests
```

Expected: FAIL to compile — `cannot find 'VersionComparison' in scope`, `cannot find type 'AppVersion' in scope`, `cannot find type 'VersionError' in scope`.

- [ ] **Step 5: Write the minimal implementation**

Create `macos/Sources/TCUpdates/AppVersion.swift`:

```swift
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
```

- [ ] **Step 6: Run the test to verify it passes**

```bash
swift test --package-path macos --filter AppVersionTests
```

Expected: PASS, 7 tests, `Executed 7 tests, with 0 failures`.

- [ ] **Step 7: Commit**

```bash
git add macos/Package.swift macos/Sources/TCUpdates macos/Tests/TCUpdatesTests \
  tests/fixtures/update-conformance/version-comparison.json
git commit -m "Compare app versions in Swift against the shared conformance fixture"
```

---

### Task 2: Homebrew install detection

**Files:**
- Create: `macos/Sources/TCUpdates/HomebrewDetector.swift`
- Create: `macos/Tests/TCUpdatesTests/HomebrewDetectorTests.swift`

**Interfaces:**
- Consumes: nothing. (`AppVersion` from Task 1 is not used here.)
- Produces:
  - `public struct HomebrewInstallState: Equatable, Sendable` with `public let isManaged: Bool`, `public let caskName: String`, `public let caskroomPath: String?`, `public var upgradeCommand: String`, `public init(isManaged: Bool, caskName: String, caskroomPath: String?)`.
  - `public enum HomebrewDetector` with `public static let caskName = "trace-commons"`, `public static let defaultPrefixes: [String]`, `public static func detect(prefixes: [String] = HomebrewDetector.defaultPrefixes, fileManager: FileManager = .default) -> HomebrewInstallState`.

Local path check only — no network, no shelling out to `brew`. Shelling out would mean the answer depends on `PATH` and on a subprocess that can hang, and this decision runs on the launch path.

- [ ] **Step 1: Write the failing test**

Create `macos/Tests/TCUpdatesTests/HomebrewDetectorTests.swift`:

```swift
import Foundation
import XCTest

@testable import TCUpdates

final class HomebrewDetectorTests: XCTestCase {
    private var root: URL!

    override func setUpWithError() throws {
        root = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("tc-homebrew-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: root)
    }

    /// Creates `<root>/<prefix>/Caskroom/<cask>` and returns `<root>/<prefix>`.
    @discardableResult
    private func makeCaskroom(prefix: String, cask: String) throws -> String {
        let dir = root.appendingPathComponent(prefix).appendingPathComponent("Caskroom")
            .appendingPathComponent(cask)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return root.appendingPathComponent(prefix).path
    }

    private func prefixPaths(_ names: [String]) -> [String] {
        names.map { root.appendingPathComponent($0).path }
    }

    func testNoCaskroomMeansWeInstalledItAndMayUpdateOurselves() {
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertFalse(state.isManaged)
        XCTAssertNil(state.caskroomPath)
        XCTAssertEqual(state.caskName, "trace-commons")
    }

    func testAppleSiliconPrefixIsDetected() throws {
        let prefix = try makeCaskroom(prefix: "opt/homebrew", cask: "trace-commons")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertTrue(state.isManaged)
        XCTAssertEqual(state.caskroomPath, prefix + "/Caskroom/trace-commons")
    }

    func testIntelPrefixIsDetected() throws {
        let prefix = try makeCaskroom(prefix: "usr/local", cask: "trace-commons")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertTrue(state.isManaged)
        XCTAssertEqual(state.caskroomPath, prefix + "/Caskroom/trace-commons")
    }

    func testAnUnrelatedCaskDoesNotCountAsOurs() throws {
        try makeCaskroom(prefix: "opt/homebrew", cask: "some-other-app")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertFalse(state.isManaged)
    }

    func testAFileWhereTheCaskroomEntryShouldBeIsNotAnInstall() throws {
        let dir = root.appendingPathComponent("opt/homebrew/Caskroom")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data().write(to: dir.appendingPathComponent("trace-commons"))
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertFalse(state.isManaged, "a plain file is not a Caskroom entry")
    }

    func testTheUpgradeCommandIsExactlyWhatWeTellPeopleToRun() {
        let state = HomebrewInstallState(
            isManaged: true, caskName: "trace-commons",
            caskroomPath: "/opt/homebrew/Caskroom/trace-commons"
        )
        XCTAssertEqual(state.upgradeCommand, "brew upgrade --cask trace-commons")
    }

    func testTheShippingPrefixesAreBothHomebrewLocations() {
        XCTAssertEqual(HomebrewDetector.defaultPrefixes, ["/opt/homebrew", "/usr/local"])
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
swift test --package-path macos --filter HomebrewDetectorTests
```

Expected: FAIL to compile — `cannot find 'HomebrewDetector' in scope`, `cannot find type 'HomebrewInstallState' in scope`.

- [ ] **Step 3: Write the minimal implementation**

Create `macos/Sources/TCUpdates/HomebrewDetector.swift`:

```swift
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
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
swift test --package-path macos --filter HomebrewDetectorTests
```

Expected: PASS, 7 tests, `Executed 7 tests, with 0 failures`.

- [ ] **Step 5: Commit**

```bash
git add macos/Sources/TCUpdates/HomebrewDetector.swift \
  macos/Tests/TCUpdatesTests/HomebrewDetectorTests.swift
git commit -m "Detect a Homebrew cask install from the Caskroom path"
```

---

### Task 3: The update policy that decides whether Sparkle runs at all

**Files:**
- Create: `macos/Sources/TCUpdates/UpdatePolicy.swift`
- Create: `macos/Tests/TCUpdatesTests/UpdatePolicyTests.swift`

**Interfaces:**
- Consumes: `HomebrewInstallState` (Task 2).
- Produces:
  - `public enum UpdateMode: Equatable, Sendable { case selfUpdating; case managedByHomebrew(upgradeCommand: String); case disabled(reason: String) }`
  - `public enum UpdatePolicy` with `public static let noFeedReason = "update_feed_not_configured"`, `public static let insecureFeedReason = "update_feed_not_https"`, and `public static func mode(homebrew: HomebrewInstallState, feedURL: String?) -> UpdateMode`.

This is a separate task from Task 2 because it is a separate decision, and it is the one that has to be right: `UpdateController` (Task 8) starts Sparkle if and only if this returns `.selfUpdating`. Keeping it here means the "never start Sparkle under Homebrew" rule is covered by a unit test rather than by reading the startup path.

- [ ] **Step 1: Write the failing test**

Create `macos/Tests/TCUpdatesTests/UpdatePolicyTests.swift`:

```swift
import Foundation
import XCTest

@testable import TCUpdates

final class UpdatePolicyTests: XCTestCase {
    private let feed = "https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml"

    private func managed() -> HomebrewInstallState {
        HomebrewInstallState(
            isManaged: true, caskName: "trace-commons",
            caskroomPath: "/opt/homebrew/Caskroom/trace-commons"
        )
    }

    private func unmanaged() -> HomebrewInstallState {
        HomebrewInstallState(isManaged: false, caskName: "trace-commons", caskroomPath: nil)
    }

    func testADragInstalledAppWithAFeedSelfUpdates() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: feed),
            .selfUpdating
        )
    }

    func testAHomebrewInstallDefersAndCarriesTheCommand() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: managed(), feedURL: feed),
            .managedByHomebrew(upgradeCommand: "brew upgrade --cask trace-commons")
        )
    }

    func testHomebrewWinsEvenWhenNoFeedIsConfigured() {
        // A Homebrew install must never reach the Sparkle branch, and must
        // never be told "updates are unavailable" when the real answer is
        // "brew owns this".
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: managed(), feedURL: nil),
            .managedByHomebrew(upgradeCommand: "brew upgrade --cask trace-commons")
        )
    }

    func testAMissingFeedDisablesUpdatesRatherThanStartingSparkleBlind() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: nil),
            .disabled(reason: UpdatePolicy.noFeedReason)
        )
    }

    func testAnEmptyFeedStringCountsAsMissing() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: ""),
            .disabled(reason: UpdatePolicy.noFeedReason)
        )
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: "   "),
            .disabled(reason: UpdatePolicy.noFeedReason)
        )
    }

    func testANonHttpsFeedIsRefused() {
        // The appcast authorizes an install. Fetching it over a transport
        // anybody on the path can rewrite is not a downgrade in security to
        // be weighed -- it is the whole control gone.
        XCTAssertEqual(
            UpdatePolicy.mode(
                homebrew: unmanaged(),
                feedURL: "http://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml"
            ),
            .disabled(reason: UpdatePolicy.insecureFeedReason)
        )
    }

    func testTheDisabledReasonsAreStableLabelsSafeToLog() {
        XCTAssertEqual(UpdatePolicy.noFeedReason, "update_feed_not_configured")
        XCTAssertEqual(UpdatePolicy.insecureFeedReason, "update_feed_not_https")
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
swift test --package-path macos --filter UpdatePolicyTests
```

Expected: FAIL to compile — `cannot find 'UpdatePolicy' in scope`, `cannot find type 'UpdateMode' in scope`.

- [ ] **Step 3: Write the minimal implementation**

Create `macos/Sources/TCUpdates/UpdatePolicy.swift`:

```swift
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
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
swift test --package-path macos --filter UpdatePolicyTests
```

Expected: PASS, 7 tests, `Executed 7 tests, with 0 failures`.

- [ ] **Step 5: Commit**

```bash
git add macos/Sources/TCUpdates/UpdatePolicy.swift \
  macos/Tests/TCUpdatesTests/UpdatePolicyTests.swift
git commit -m "Decide from Homebrew and feed state whether Sparkle may run"
```

---

### Task 4: Add Sparkle as a pinned SwiftPM dependency

**Files:**
- Modify: `macos/Package.swift:21-24` (add a `dependencies:` array to `Package(...)`) and the `TraceCommonsApp` target's `dependencies:` (currently line 36)
- Create: `macos/Package.resolved` (generated, then committed)

**Interfaces:**
- Consumes: `TCUpdates` (Task 1) — the app target now depends on it.
- Produces: the `Sparkle` product importable from `TraceCommonsApp`, and the extracted XCFramework under `macos/.build/artifacts/sparkle/Sparkle/Sparkle.xcframework`, which Task 6 copies into the bundle.

Sparkle's SwiftPM package is a single `binaryTarget` pointing at `Sparkle-for-Swift-Package-Manager.zip`. There is no source build and no transitive dependency. SwiftPM extracts the zip into `.build/artifacts/<package-identity>/<target-name>/`, which for this package is `.build/artifacts/sparkle/Sparkle/`, containing `Sparkle.xcframework` and a `bin/` directory holding `generate_keys`, `sign_update`, and `generate_appcast`.

- [ ] **Step 1: Add the dependency to Package.swift**

In `macos/Package.swift`, change the `Package(` initializer. It currently reads:

```swift
let package = Package(
    name: "TraceCommons",
    platforms: [.macOS(.v14)],
    targets: [
```

Change it to:

```swift
let package = Package(
    name: "TraceCommons",
    platforms: [.macOS(.v14)],
    dependencies: [
        // Pinned exactly, not by range. This dependency decides which bytes
        // replace the running app, so "whatever resolves today" is not an
        // acceptable answer; a bump is a reviewed change with a new
        // Package.resolved in the diff.
        //
        // Sparkle is a single binaryTarget (an XCFramework zip) with no
        // transitive dependencies. Its adoption is a recorded waiver of the
        // repo's dependency policy -- see
        // docs/superpowers/specs/2026-08-17-desktop-auto-update-design.md.
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.9.6")
    ],
    targets: [
```

Then change the `TraceCommonsApp` target's dependencies from:

```swift
        .executableTarget(
            name: "TraceCommonsApp",
            dependencies: ["TCBridge"],
```

to:

```swift
        .executableTarget(
            name: "TraceCommonsApp",
            dependencies: [
                "TCBridge",
                "TCUpdates",
                .product(name: "Sparkle", package: "Sparkle"),
            ],
```

Leave `tc-ffi-demo` untouched: it is a milestone demo of the C ABI and has no business linking an updater.

- [ ] **Step 2: Resolve and verify the dependency graph**

```bash
swift package --package-path macos resolve
cat macos/Package.resolved
```

Expected: `Package.resolved` names exactly one dependency, `sparkle`, at version `2.9.6`, with a `location` of `https://github.com/sparkle-project/Sparkle`. If any other package appears, stop — that violates the "no other new dependency" constraint.

- [ ] **Step 3: Confirm the artifact layout the build scripts will depend on**

```bash
find macos/.build/artifacts -maxdepth 4 -name 'Sparkle.xcframework'
ls macos/.build/artifacts/sparkle/Sparkle/bin
ls macos/.build/artifacts/sparkle/Sparkle/Sparkle.xcframework
```

Expected: exactly one `Sparkle.xcframework` path; `bin` contains `generate_keys`, `sign_update` and `generate_appcast`; the xcframework contains `Info.plist` and a `macos-arm64_x86_64` directory.

If `bin` is missing, run `swift build --package-path macos` first — SwiftPM extracts the artifact lazily.

- [ ] **Step 4: Build universally and confirm the app links Sparkle by rpath**

```bash
cargo build -p trace-commons-contributor-ffi
swift build --package-path macos --arch arm64 --arch x86_64
otool -L macos/.build/apple/Products/Debug/TraceCommonsApp | grep -i sparkle
```

Expected: exactly one line, `@rpath/Sparkle.framework/Versions/B/Sparkle (compatibility version 1.0.0, current version ...)`.

`@rpath` is why Task 6 works at all: `make-app-bundle.sh` already adds `@executable_path/../Frameworks` to the executable's rpath list for the FFI dylib, so a framework copied into `Contents/Frameworks` is found with no further `install_name_tool` work.

- [ ] **Step 5: Confirm the tests still pass with the dependency present**

```bash
swift test --package-path macos --filter TCUpdatesTests
```

Expected: PASS, 21 tests across `AppVersionTests`, `HomebrewDetectorTests` and `UpdatePolicyTests`. `TCUpdates` does not import Sparkle, so these still run without any framework on the runtime search path.

- [ ] **Step 6: Commit**

```bash
git add macos/Package.swift macos/Package.resolved
git commit -m "Pin Sparkle 2.9.6 as a binary SwiftPM dependency of the macOS app"
```

---

### Task 5: Sparkle keys in the Info.plist, fail-closed when no key is supplied

**Files:**
- Modify: `macos/scripts/info-plist.sh:12-34`
- Modify: `macos/scripts/make-release-dmg.sh:81-85` (add the key to the required-env loop) and its header comment block at lines 36-44
- Modify: `.github/workflows/release-apps.yml` (the `Build, sign, notarize and staple` step's `env:` block, currently lines 120-128)
- Modify: `crates/trace-commons-contributor/tests/release_pipeline.rs` (append tests)

**Interfaces:**
- Consumes: the `SUFeedURL` value published by `scripts/updates/generate-appcast.sh` (the manifest-publishing plan, Task 4).
- Produces: an `Info.plist` carrying `SUFeedURL`, `SUPublicEDKey`, `SUEnableAutomaticChecks`, `SUAutomaticallyUpdate` and `SUScheduledCheckInterval` — read at runtime by `UpdateController` (Task 8) and by Sparkle itself.

`SUEnableAutomaticChecks` `true` + `SUAutomaticallyUpdate` `false` is the approved pairing: Sparkle checks in the background on its own without ever asking permission to check, and no bytes are replaced until a person says yes.

- [ ] **Step 1: Write the failing tests**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
/// Runs info-plist.sh with a given TC_SPARKLE_PUBLIC_ED_KEY (None = unset).
fn info_plist_with_key(key: Option<&str>) -> String {
    let script = repo_root().join("macos/scripts/info-plist.sh");
    let mut command = Command::new("bash");
    command.arg(&script).args(["0.4.2", "17"]);
    match key {
        Some(value) => {
            command.env("TC_SPARKLE_PUBLIC_ED_KEY", value);
        }
        None => {
            command.env_remove("TC_SPARKLE_PUBLIC_ED_KEY");
        }
    }
    let output = command.output().expect("failed to run info-plist.sh");
    assert!(
        output.status.success(),
        "info-plist.sh failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn info_plist_carries_the_approved_sparkle_configuration() {
    let plist = info_plist_with_key(Some("dGVzdC1wdWJsaWMta2V5LWJhc2U2NC12YWx1ZQ=="));

    assert!(
        plist.contains(
            "<key>SUFeedURL</key><string>\
             https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml</string>"
        ),
        "the appcast feed URL is wrong or missing:\n{plist}"
    );
    assert!(
        plist.contains(
            "<key>SUPublicEDKey</key><string>dGVzdC1wdWJsaWMta2V5LWJhc2U2NC12YWx1ZQ==</string>"
        ),
        "the EdDSA public key was not injected:\n{plist}"
    );
    // Checks on, install off. Sparkle checks in the background without ever
    // asking permission to check, and nothing is replaced until a person
    // says yes. Flipping SUAutomaticallyUpdate to true would make this app
    // swap its own bytes silently, which the design forbids.
    assert!(
        plist.contains("<key>SUEnableAutomaticChecks</key><true/>"),
        "automatic checks are not enabled:\n{plist}"
    );
    assert!(
        plist.contains("<key>SUAutomaticallyUpdate</key><false/>"),
        "automatic install must stay off:\n{plist}"
    );
    assert!(
        plist.contains("<key>SUScheduledCheckInterval</key><integer>86400</integer>"),
        "the daily check interval is wrong or missing:\n{plist}"
    );
}

#[test]
fn info_plist_ships_no_feed_at_all_without_a_public_key() {
    // Fail closed. A bundle with a feed but no key would ask Sparkle to
    // fetch an appcast it cannot authenticate; a bundle with neither simply
    // has no update path, which is the correct state for a dev build.
    let plist = info_plist_with_key(None);
    assert!(
        !plist.contains("SUFeedURL"),
        "a keyless bundle must not carry a feed URL:\n{plist}"
    );
    assert!(
        !plist.contains("SUPublicEDKey"),
        "a keyless bundle must not carry an empty key:\n{plist}"
    );
    assert!(
        plist.contains("<key>SUEnableAutomaticChecks</key><false/>"),
        "a keyless bundle must not enable automatic checks:\n{plist}"
    );
}

#[test]
fn the_release_script_refuses_without_the_sparkle_public_key() {
    let script = read("macos/scripts/make-release-dmg.sh");
    assert!(
        script.contains("TC_SPARKLE_PUBLIC_ED_KEY"),
        "make-release-dmg.sh must require TC_SPARKLE_PUBLIC_ED_KEY. A release \
         built without it ships an app that can never receive an update, and \
         nothing about the DMG would look wrong."
    );
}

#[test]
fn the_release_workflow_passes_the_sparkle_public_key() {
    let workflow = read(".github/workflows/release-apps.yml");
    assert!(
        workflow.contains("TC_SPARKLE_PUBLIC_ED_KEY: ${{ secrets.SPARKLE_PUBLIC_ED_KEY }}"),
        "the macOS release job must pass the Sparkle public key through"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
```

Expected: FAIL — 4 new failures, e.g. `the appcast feed URL is wrong or missing`, `a keyless bundle must not enable automatic checks` (the current script emits neither key), and `make-release-dmg.sh must require TC_SPARKLE_PUBLIC_ED_KEY`.

- [ ] **Step 3: Rewrite info-plist.sh**

Replace the whole of `macos/scripts/info-plist.sh` with:

```bash
#!/usr/bin/env bash
# Print the Info.plist for TraceCommons.app.
#
# Extracted from make-app-bundle.sh so the version can be injected from a
# release tag and asserted in a test without a Swift toolchain. The old
# heredoc hardcoded CFBundleShortVersionString to 0.1.0, which meant any
# tagged release would have shipped a DMG claiming 0.1.0 -- and Homebrew
# compares a cask's declared version against what is installed, so that also
# broke `brew upgrade`.
#
# # Sparkle
#
# The updater's configuration lives here rather than in Swift because Sparkle
# reads it from the bundle before any of our code runs.
#
# TC_SPARKLE_PUBLIC_ED_KEY is the base64 EdDSA public key from Sparkle's
# generate_keys. When it is unset -- the development case -- this script
# emits NO feed URL, NO key, and automatic checks off. That is deliberate and
# it is the fail-closed direction: a bundle that carried a feed without a key
# would be asking Sparkle to fetch an appcast it cannot authenticate. A
# release cannot reach that state because make-release-dmg.sh refuses to run
# without the key.
set -euo pipefail

SHORT_VERSION="${1:?usage: info-plist.sh <short_version> <build_version>}"
BUILD_VERSION="${2:?usage: info-plist.sh <short_version> <build_version>}"

# The published appcast, written by scripts/updates/generate-appcast.sh into
# the same public bucket as the flatpak repo. HTTPS is not decoration here:
# the appcast is what authorizes an install.
SPARKLE_FEED_URL="https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml"

SPARKLE_KEYS=""
if [ -n "${TC_SPARKLE_PUBLIC_ED_KEY:-}" ]; then
  SPARKLE_KEYS="$(cat <<KEYS
    <key>SUFeedURL</key><string>${SPARKLE_FEED_URL}</string>
    <key>SUPublicEDKey</key><string>${TC_SPARKLE_PUBLIC_ED_KEY}</string>
    <!-- Check automatically, never install automatically. Sparkle looks for
         an update on launch and once a day thereafter without asking
         permission to look; the bytes on disk do not change until a person
         says yes. -->
    <key>SUEnableAutomaticChecks</key><true/>
    <key>SUAutomaticallyUpdate</key><false/>
    <key>SUScheduledCheckInterval</key><integer>86400</integer>
KEYS
)"
else
  SPARKLE_KEYS="    <key>SUEnableAutomaticChecks</key><false/>"
fi

cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Trace Commons</string>
    <key>CFBundleDisplayName</key><string>Trace Commons</string>
    <key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>
    <key>CFBundleExecutable</key><string>TraceCommonsApp</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${SHORT_VERSION}</string>
    <key>CFBundleVersion</key><string>${BUILD_VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHumanReadableCopyright</key><string>Trace Commons</string>
    <!-- Menu-bar item, no Dock icon: the shape macOS users expect from a
         background utility. -->
    <key>LSUIElement</key><true/>
${SPARKLE_KEYS}
</dict>
</plist>
PLIST
```

- [ ] **Step 4: Require the key in the release path**

In `macos/scripts/make-release-dmg.sh`, add to the credential list in the header comment, after the `MACOS_NOTARY_ASC_ISSUER_ID` line (currently line 43):

```bash
#   TC_SPARKLE_PUBLIC_ED_KEY        base64 EdDSA public key from Sparkle's
#                                   generate_keys. Without it the bundle ships
#                                   no feed URL, and the released app could
#                                   never receive an update -- a failure that
#                                   is invisible in the DMG itself.
```

Then extend the required-variable loop (currently lines 81-85) to:

```bash
for var in MACOS_CERTIFICATE_P12_BASE64 MACOS_CERTIFICATE_PASSWORD \
           MACOS_SIGNING_IDENTITY MACOS_NOTARY_ASC_KEY_P8_BASE64 \
           MACOS_NOTARY_ASC_KEY_ID MACOS_NOTARY_ASC_ISSUER_ID \
           TC_SPARKLE_PUBLIC_ED_KEY; do
  require_env "$var"
done
```

- [ ] **Step 5: Pass the key from the release workflow**

In `.github/workflows/release-apps.yml`, in the macOS job's `Build, sign, notarize and staple` step, add one line to the `env:` block after `MACOS_NOTARY_ASC_ISSUER_ID`:

```yaml
          TC_SPARKLE_PUBLIC_ED_KEY: ${{ secrets.SPARKLE_PUBLIC_ED_KEY }}
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
```

Expected: PASS, all tests including the four new ones.

Then confirm the plist is well-formed both ways:

```bash
TC_SPARKLE_PUBLIC_ED_KEY=dGVzdA== ./macos/scripts/info-plist.sh 0.4.2 17 | plutil -lint -
./macos/scripts/info-plist.sh 0.4.2 17 | plutil -lint -
```

Expected: `-: OK` twice.

- [ ] **Step 7: Commit**

```bash
git add macos/scripts/info-plist.sh macos/scripts/make-release-dmg.sh \
  .github/workflows/release-apps.yml \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Configure Sparkle in the Info.plist and refuse a release without its key"
```

---

### Task 6: Embed Sparkle.framework into the bundle and ad-hoc sign it inside-out

**Files:**
- Modify: `macos/scripts/make-app-bundle.sh:85-113`
- Modify: `crates/trace-commons-contributor/tests/release_pipeline.rs` (append tests)

**Interfaces:**
- Consumes: `macos/.build/artifacts/sparkle/Sparkle/Sparkle.xcframework` (Task 4).
- Produces: `TraceCommons.app/Contents/Frameworks/Sparkle.framework`, ad-hoc signed inside-out, ready for `make-release-dmg.sh` (Task 7) to re-sign with the Developer ID identity.

This is the highest-risk change in the plan. SwiftPM links the framework but does **not** embed it — that is Xcode's "Embed & Sign" build phase, which this hand-assembled bundle does not have. Without the copy step the app builds, signs, notarizes, and then crashes on launch with `Library not loaded: @rpath/Sparkle.framework/Versions/B/Sparkle`.

Three mechanics matter and each has a specific failure mode:

1. **Copy with `ditto`, not `cp`.** A framework is a versioned bundle: `Sparkle.framework/Sparkle`, `Versions/Current`, `Resources` and `Headers` are all symlinks into `Versions/B`. `ditto` reproduces them; a copy that dereferences them produces a bundle `codesign` rejects as malformed.
2. **Take the `macos-arm64_x86_64` slice.** The XCFramework holds slices per platform. Picking the wrong directory, or letting a future Sparkle release rename it, must be a loud failure rather than a thin binary that ships and fails to launch on half of the installed base — the exact hazard `verify_universal` already exists to catch.
3. **Sign inside-out and never with `--deep`.** Order is XPC services (Downloader with `--preserve-metadata=entitlements`), then `Autoupdate`, then `Updater.app`, then `Sparkle.framework`, then the app. `--deep` re-signs the Downloader XPC service without its entitlements and is the most common Sparkle signing failure there is.

- [ ] **Step 1: Write the failing tests**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn the_bundle_script_embeds_sparkle_with_ditto() {
    let script = read("macos/scripts/make-app-bundle.sh");
    assert!(
        script.contains("Sparkle.xcframework"),
        "make-app-bundle.sh must locate the Sparkle XCFramework. SwiftPM \
         links it but never embeds it; without a copy step the signed, \
         notarized app crashes on launch with 'Library not loaded'."
    );
    assert!(
        script.contains("macos-arm64_x86_64"),
        "the universal XCFramework slice must be named explicitly"
    );
    assert!(
        script.contains("ditto"),
        "a framework must be copied with ditto: Versions/Current, Resources \
         and the top-level binary are symlinks, and a copy that dereferences \
         them produces a bundle codesign rejects"
    );
}

#[test]
fn no_script_signs_sparkle_with_deep() {
    for path in [
        "macos/scripts/make-app-bundle.sh",
        "macos/scripts/make-release-dmg.sh",
    ] {
        let script = read(path);
        for line in script.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                continue;
            }
            if trimmed.contains("codesign") && trimmed.contains("--sign") {
                assert!(
                    !line.contains("--deep"),
                    "{path}: codesign --sign --deep re-signs Sparkle's Downloader \
                     XPC service without its entitlements. Sign inside-out \
                     instead. Offending line: {line}"
                );
            }
        }
    }
}

#[test]
fn the_bundle_script_signs_sparkle_inside_out() {
    let script = read("macos/scripts/make-app-bundle.sh");
    let order = [
        "XPCServices/Installer.xpc",
        "XPCServices/Downloader.xpc",
        "Versions/B/Autoupdate",
        "Versions/B/Updater.app",
    ];
    let mut previous = 0usize;
    for needle in order {
        let at = script
            .find(needle)
            .unwrap_or_else(|| panic!("make-app-bundle.sh never mentions {needle}"));
        assert!(
            at > previous,
            "{needle} is signed out of order; nested code must be signed \
             before the framework that seals it"
        );
        previous = at;
    }
    assert!(
        script.contains("--preserve-metadata=entitlements"),
        "Downloader.xpc must be signed with --preserve-metadata=entitlements \
         (Sparkle >= 2.6), or it loses the entitlement it needs"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
```

Expected: FAIL — `make-app-bundle.sh must locate the Sparkle XCFramework` and `make-app-bundle.sh never mentions XPCServices/Installer.xpc`. `no_script_signs_sparkle_with_deep` passes already; that is correct — it is a regression guard.

- [ ] **Step 3: Add the embedding step to make-app-bundle.sh**

In `macos/scripts/make-app-bundle.sh`, insert this block immediately after the `install_name_tool -add_rpath ...` line (currently line 99) and before the ad-hoc signing block:

```bash
# --- Sparkle ---------------------------------------------------------------
#
# SwiftPM LINKS Sparkle but does not EMBED it. In an Xcode project that is
# the "Embed & Sign" build phase; this bundle is assembled by hand, so the
# copy has to happen here. Skipping it produces an app that builds, signs and
# notarizes cleanly and then dies on launch with
#   Library not loaded: @rpath/Sparkle.framework/Versions/B/Sparkle
# The rpath added just above is what makes @rpath resolve to Frameworks/.
SPARKLE_MATCHES=()
while IFS= read -r match; do
  SPARKLE_MATCHES+=("$match")
done < <(find "$PACKAGE_DIR/.build/artifacts" -maxdepth 4 -type d -name 'Sparkle.xcframework' 2>/dev/null)

if [ "${#SPARKLE_MATCHES[@]}" -ne 1 ]; then
  echo "FATAL: expected exactly one Sparkle.xcframework under .build/artifacts," >&2
  echo "found ${#SPARKLE_MATCHES[@]}. Run 'swift package resolve' first." >&2
  printf '  %s\n' ${SPARKLE_MATCHES[@]+"${SPARKLE_MATCHES[@]}"} >&2
  exit 1
fi
SPARKLE_XCFRAMEWORK="${SPARKLE_MATCHES[0]}"

# The XCFramework carries one directory per platform slice. Naming the slice
# explicitly means a Sparkle release that renames or splits it fails here,
# loudly, instead of shipping a thin framework that passes signing and
# notarization and then fails to launch on whichever architecture was left
# out -- the same hazard verify_universal already guards for our own code.
SPARKLE_SLICE="$SPARKLE_XCFRAMEWORK/macos-arm64_x86_64/Sparkle.framework"
if [ ! -d "$SPARKLE_SLICE" ]; then
  echo "FATAL: no macos-arm64_x86_64 slice in $SPARKLE_XCFRAMEWORK" >&2
  echo "Slices present:" >&2
  ls -1 "$SPARKLE_XCFRAMEWORK" >&2
  exit 1
fi

# ditto, not cp: Sparkle.framework/Sparkle, Versions/Current, Resources and
# Headers are symlinks into Versions/B. A copy that dereferences them yields
# a bundle codesign rejects as malformed.
rm -rf "$APP/Contents/Frameworks/Sparkle.framework"
ditto "$SPARKLE_SLICE" "$APP/Contents/Frameworks/Sparkle.framework"

SPARKLE_FRAMEWORK="$APP/Contents/Frameworks/Sparkle.framework"
verify_universal "Sparkle framework" "$SPARKLE_FRAMEWORK/Versions/B/Sparkle"
```

- [ ] **Step 4: Replace the ad-hoc signing block with inside-out signing**

In the same file, replace the existing signing block (currently lines 105-108):

```bash
if [ "${TC_SKIP_ADHOC_SIGN:-0}" != "1" ]; then
  codesign --force --sign - --timestamp=none "$APP/Contents/Frameworks/$DYLIB_NAME" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 || true
fi
```

with:

```bash
# An ad-hoc signature is what makes a DEVELOPMENT bundle launchable. The
# release path signs with a Developer ID immediately afterwards, so doing it
# here first is wasted work that also makes the release path read as if it
# might ship an ad-hoc signature.
#
# The ORDER below is the same order make-release-dmg.sh uses, and it is not
# arbitrary: codesign seals nested code, so anything inside must be signed
# before the thing that contains it. `--deep` is never used -- it re-signs
# Sparkle's Downloader XPC service without its entitlements, which is the
# single most common way a Sparkle integration breaks.
if [ "${TC_SKIP_ADHOC_SIGN:-0}" != "1" ]; then
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Installer.xpc" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none --preserve-metadata=entitlements \
    "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Downloader.xpc" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK/Versions/B/Autoupdate" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK/Versions/B/Updater.app" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none \
    "$APP/Contents/Frameworks/$DYLIB_NAME" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 || true
fi
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
```

Expected: PASS, all tests.

- [ ] **Step 6: Verify the real bundle, since a script test cannot**

```bash
cargo build -p trace-commons-contributor-ffi
./macos/scripts/make-app-bundle.sh debug 0.0.0-dev 1
```

Expected in the output: `verified universal (arm64 x86_64): Sparkle framework`.

Then, each of the following must produce the stated result:

```bash
ls -l macos/.build/TraceCommons.app/Contents/Frameworks/Sparkle.framework
```
Expected: `Sparkle`, `Resources`, `Headers`, `Modules` present as **symlinks** (`l` in the mode column) alongside a real `Versions` directory. If they are regular files or directories, `ditto` was not used.

```bash
ls macos/.build/TraceCommons.app/Contents/Frameworks/Sparkle.framework/Versions/B
```
Expected: `Autoupdate`, `Modules`, `Resources`, `Sparkle`, `Updater.app`, `XPCServices`.

```bash
codesign -dv --verbose=4 \
  macos/.build/TraceCommons.app/Contents/Frameworks/Sparkle.framework 2>&1 | \
  grep -E 'Identifier|Signature|Format'
```
Expected: `Identifier=org.sparkle-project.Sparkle`, `Signature=adhoc`, and a `Format=bundle with ...` line.

```bash
codesign --verify --deep --strict --verbose=2 macos/.build/TraceCommons.app
```
Expected: `macos/.build/TraceCommons.app: valid on disk` and `macos/.build/TraceCommons.app: satisfies its Designated Requirement`. (`--deep` here is verification, which is fine and is what the existing release script already does; only `codesign --sign --deep` is forbidden.)

```bash
otool -L macos/.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp | grep -i sparkle
```
Expected: `@rpath/Sparkle.framework/Versions/B/Sparkle`.

```bash
otool -l macos/.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp | grep -A2 LC_RPATH
```
Expected: a `path @executable_path/../Frameworks` entry.

- [ ] **Step 7: Commit**

```bash
git add macos/scripts/make-app-bundle.sh \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Embed Sparkle.framework in the app bundle and sign it inside-out"
```

---

### Task 7: Developer ID signing for Sparkle in the release path

**Files:**
- Modify: `macos/scripts/make-release-dmg.sh:148-164`
- Modify: `crates/trace-commons-contributor/tests/release_pipeline.rs` (append one test)

**Interfaces:**
- Consumes: the embedded `Sparkle.framework` from Task 6.
- Produces: a Developer ID signed, hardened-runtime, notarized and stapled DMG whose nested Sparkle code all carries a secure timestamp.

The ad-hoc signatures from Task 6 do not survive into a release: `make-release-dmg.sh` runs `make-app-bundle.sh` with `TC_SKIP_ADHOC_SIGN=1`, so the framework arrives in the bundle carrying Sparkle's own upstream ad-hoc signature and must be re-signed here. Every nested component needs `--timestamp` and `--options runtime`, because notarization rejects any nested Mach-O without a secure timestamp and hardened runtime — and it reports that as a failure of the whole submission, not of the one file.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn the_release_script_signs_every_sparkle_component_for_notarization() {
    let script = read("macos/scripts/make-release-dmg.sh");
    for needle in [
        "XPCServices/Installer.xpc",
        "XPCServices/Downloader.xpc",
        "Versions/B/Autoupdate",
        "Versions/B/Updater.app",
        "--preserve-metadata=entitlements",
    ] {
        assert!(
            script.contains(needle),
            "make-release-dmg.sh must sign {needle}. Notarization rejects the \
             whole submission when any nested Mach-O lacks a Developer ID \
             signature, a secure timestamp, or the hardened runtime."
        );
    }
    // Everything nested must be signed before the app bundle that seals it.
    let last_sparkle = script
        .rfind("Sparkle.framework")
        .expect("make-release-dmg.sh never mentions Sparkle.framework");
    let outer_sign = script
        .find("--sign \"$MACOS_SIGNING_IDENTITY\" \"$APP\"")
        .expect("the outer app signing call changed shape");
    assert!(
        last_sparkle < outer_sign,
        "Sparkle is signed after the app bundle; signing the outer bundle \
         first is invalidated the moment anything inside it is touched"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test -p trace-commons-contributor --test release_pipeline \
  the_release_script_signs_every_sparkle_component_for_notarization
```

Expected: FAIL — `make-release-dmg.sh must sign XPCServices/Installer.xpc`.

- [ ] **Step 3: Add the signing block**

In `macos/scripts/make-release-dmg.sh`, replace the signing section (currently lines 148-164) with:

```bash
echo "--- signing"
# Nested code is signed before the bundle that contains it: codesign seals
# what is inside, so signing the outer bundle first would be invalidated by
# touching anything inner afterwards.
#
# `--deep` is deliberately absent and must stay absent. It would re-sign
# Sparkle's Downloader XPC service without the entitlement it ships with,
# which is the single most common way a Sparkle integration breaks. Sparkle's
# own documentation gives exactly this ordering.
SPARKLE_FRAMEWORK="$APP/Contents/Frameworks/Sparkle.framework"
if [ ! -d "$SPARKLE_FRAMEWORK" ]; then
  echo "refusing to build a release: Sparkle.framework is not in the bundle." >&2
  echo "make-app-bundle.sh embeds it; a bundle without it builds, signs and" >&2
  echo "notarizes cleanly and then crashes on launch." >&2
  exit 1
fi

codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Installer.xpc"
# Sparkle >= 2.6 ships Downloader.xpc with its own entitlements; re-signing
# without preserving them removes the network access it needs.
codesign --force --timestamp --options runtime --preserve-metadata=entitlements \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Downloader.xpc"
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/Autoupdate"
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/Updater.app"
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK"

# The embedded dylib is signed before the bundle that contains it, for the
# same reason.
find "$APP/Contents/Frameworks" -name '*.dylib' -print0 |
  while IFS= read -r -d '' dylib; do
    codesign --force --timestamp --options runtime \
      --sign "$MACOS_SIGNING_IDENTITY" "$dylib"
  done

# Hardened runtime is required for notarization. There is deliberately no
# entitlements file: this app needs no exception to the hardened runtime, and
# adding entitlements it does not use would widen what a compromised process
# could do for no benefit. Sparkle's updater runs out of process precisely so
# that the app does not need one.
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
```

Expected: PASS, all tests.

- [ ] **Step 5: Verify the signing order against a real bundle without release credentials**

The release script cannot be run without a Developer ID key. Exercise the same ordering against the ad-hoc bundle instead, which proves the paths exist and the sequence is signable:

```bash
cargo build -p trace-commons-contributor-ffi
./macos/scripts/make-app-bundle.sh debug 0.0.0-dev 1
codesign --verify --deep --strict --verbose=2 macos/.build/TraceCommons.app
codesign -dv --verbose=4 \
  macos/.build/TraceCommons.app/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices/Downloader.xpc 2>&1 | \
  grep -E 'Identifier|Signature'
```

Expected: the bundle verifies, and the Downloader reports `Identifier=org.sparkle-project.Downloader` with `Signature=adhoc`.

Record explicitly, in the PR body: **the Developer ID and notarization path in this task has not been executed.** `make-release-dmg.sh` has never run against real credentials (see its header). The gates that would change that are a real run producing a signed, notarized, stapled DMG, followed by opening that DMG on a Mac that did not build it, with the network off, and confirming it launches with no Gatekeeper prompt. Until both happen, do not describe this as verified.

- [ ] **Step 6: Commit**

```bash
git add macos/scripts/make-release-dmg.sh \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Sign Sparkle with the Developer ID identity before notarizing"
```

---

### Task 8: The update controller, and never starting Sparkle under Homebrew

**Files:**
- Create: `macos/Sources/TraceCommonsApp/UpdateController.swift`
- Modify: `macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift:53-92` (start it in `launch()`)

**Interfaces:**
- Consumes: `UpdatePolicy.mode(homebrew:feedURL:)`, `UpdateMode`, `HomebrewDetector.detect(prefixes:fileManager:)` (Tasks 2 and 3); the `Sparkle` product (Task 4); the `SUFeedURL` Info.plist key (Task 5).
- Produces: `@MainActor final class UpdateController: ObservableObject` with `static let shared: UpdateController`, `@Published private(set) var mode: UpdateMode`, `@Published private(set) var lastCheckDate: Date?`, `@Published private(set) var canCheckNow: Bool`, `let currentVersion: String`, `func start()`, `func checkNow()`, `func refreshLastCheckDate()`. Consumed by `SettingsView` in Task 9.

There is no unit test here: the type owns a `SPUStandardUpdaterController`, which needs a real bundle, a real framework and a running app loop. Everything in it that *can* be decided without those lives in `TCUpdates` and is tested in Tasks 1-3. The verification for this task is a launch check.

- [ ] **Step 1: Write the implementation**

Create `macos/Sources/TraceCommonsApp/UpdateController.swift`:

```swift
import Combine
import Foundation
import Sparkle
import TCUpdates

/// Owns Sparkle, or deliberately does not.
///
/// The governing rule is that whoever installed the binary owns replacing
/// it. `UpdatePolicy` decides which of us that is, from a local path check
/// with no network; this type only acts on the answer. Sparkle is
/// constructed at all only in the `.selfUpdating` case -- constructing it
/// and then declining to start it would still leave a framework in the
/// process believing it may schedule work.
@MainActor
final class UpdateController: ObservableObject {
    static let shared = UpdateController()

    @Published private(set) var mode: UpdateMode
    /// When Sparkle last looked. Nil before the first check of this install.
    @Published private(set) var lastCheckDate: Date?
    @Published private(set) var canCheckNow: Bool = false

    /// CFBundleShortVersionString. "unknown" only when running the bare
    /// SwiftPM executable outside a bundle, which is a development case.
    let currentVersion: String

    private var updaterController: SPUStandardUpdaterController?
    private var cancellables = Set<AnyCancellable>()

    init(
        bundle: Bundle = .main,
        homebrew: HomebrewInstallState = HomebrewDetector.detect()
    ) {
        self.currentVersion =
            bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ?? "unknown"
        let feedURL = bundle.object(forInfoDictionaryKey: "SUFeedURL") as? String
        self.mode = UpdatePolicy.mode(homebrew: homebrew, feedURL: feedURL)
    }

    /// Starts the updater if and only if this install is ours.
    ///
    /// Safe to call more than once; the second call is a no-op. Called from
    /// the app's single launch path.
    func start() {
        guard case .selfUpdating = mode, updaterController == nil else { return }

        // startingUpdater: true starts the scheduled checks immediately.
        // Everything about WHAT gets checked and how often comes from the
        // Info.plist (SUFeedURL, SUEnableAutomaticChecks,
        // SUAutomaticallyUpdate, SUScheduledCheckInterval) rather than from
        // runtime API calls -- Sparkle's own guidance is that the runtime
        // properties are for responding to user setting changes, not for
        // establishing defaults.
        let controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
        updaterController = controller

        controller.updater.publisher(for: \.canCheckForUpdates)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] value in self?.canCheckNow = value }
            .store(in: &cancellables)

        lastCheckDate = controller.updater.lastUpdateCheckDate
    }

    /// A user-initiated check. Does nothing in any mode but `.selfUpdating`,
    /// because in every other mode there is no updater to ask.
    func checkNow() {
        guard let controller = updaterController else { return }
        controller.updater.checkForUpdates()
        lastCheckDate = controller.updater.lastUpdateCheckDate
    }

    /// Re-reads the last check time. Sparkle updates it on its own schedule,
    /// and a Settings window left open would otherwise show a stale time.
    func refreshLastCheckDate() {
        lastCheckDate = updaterController?.updater.lastUpdateCheckDate
    }
}
```

- [ ] **Step 2: Start it from the app's launch path**

In `macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift`, inside `Launcher.launch()`, add one line immediately after `model.start()` (currently line 59):

```swift
        // Update checks begin here and nowhere else. UpdateController itself
        // decides whether Sparkle runs at all: under a Homebrew install this
        // call constructs no updater and schedules nothing.
        UpdateController.shared.start()
```

- [ ] **Step 3: Build and confirm it compiles under the app target's language mode**

```bash
cargo build -p trace-commons-contributor-ffi
swift build --package-path macos --arch arm64 --arch x86_64
```

Expected: `Build complete!` with no warnings about `Sparkle` or `TCUpdates`.

- [ ] **Step 4: Verify the Homebrew branch really prevents Sparkle from starting**

Build the bundle and launch it with a Caskroom entry in place, then without:

```bash
./macos/scripts/make-app-bundle.sh debug 0.0.0-dev 1

# Without a Caskroom entry, and with no feed configured in a dev bundle, the
# app is in .disabled -- no Sparkle process, no scheduled check.
macos/.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp &
sleep 5
pgrep -fl 'Updater.app|Autoupdate' || echo "no Sparkle helper running (expected)"
kill %1
```

Expected: `no Sparkle helper running (expected)`.

Then confirm the detector agrees with reality on this machine:

```bash
swift test --package-path macos --filter HomebrewDetectorTests
ls -d /opt/homebrew/Caskroom/trace-commons /usr/local/Caskroom/trace-commons 2>/dev/null \
  || echo "no Caskroom entry on this machine"
```

Expected: the tests pass, and the `ls` reports whichever state this machine is actually in. Task 9's verification exercises the visible Homebrew branch.

- [ ] **Step 5: Commit**

```bash
git add macos/Sources/TraceCommonsApp/UpdateController.swift \
  macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift
git commit -m "Start Sparkle only when this install is not managed by Homebrew"
```

---

### Task 9: The Settings surface for updates

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift:9-33` (add the observed controller and the section to the stack) and append the section body after the `loginItem` section (currently ending at line 96)

**Interfaces:**
- Consumes: `UpdateController.shared` (Task 8) — `mode`, `currentVersion`, `lastCheckDate`, `canCheckNow`, `checkNow()`, `refreshLastCheckDate()`; `UpdateMode` (Task 3).
- Produces: nothing consumed by later tasks. This is the last task.

Placed directly after "Startup" and before "How may your traces be used?": both are facts about this installation rather than about traces, and the consent block is the one thing on this screen that should not be reached by scrolling past an updater.

- [ ] **Step 1: Add the observed controller and place the section**

In `macos/Sources/TraceCommonsApp/Views/SettingsView.swift`, add one property after `@EnvironmentObject private var model: AppModel` (line 9):

```swift
    @ObservedObject private var updates = UpdateController.shared
```

Then change the section stack in `body` from:

```swift
                connection
                loginItem
                consent
```

to:

```swift
                connection
                loginItem
                updatesSection
                consent
```

and change the `.onAppear` modifier (line 32) from:

```swift
        .onAppear { loginItemState = LoginItemManager.currentState }
```

to:

```swift
        .onAppear {
            loginItemState = LoginItemManager.currentState
            // Sparkle moves this on its own schedule; a Settings window left
            // open would otherwise keep showing the time it had at open.
            updates.refreshLastCheckDate()
        }
```

- [ ] **Step 2: Add the section body**

Insert this into `SettingsView`, immediately after the `setLoginItem(enabled:)` method (currently ending at line 113):

```swift
    /// Version, update state, and -- when Homebrew owns this copy -- the one
    /// command that actually works.
    ///
    /// The Homebrew branch is not an apology for a missing feature. Homebrew
    /// placed these bytes and Homebrew replaces them; an app that offered a
    /// "Check Now" button here would be offering to fight the package
    /// manager over the same file.
    private var updatesSection: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: "Updates")

            HStack(spacing: TC.Space.s) {
                TCFieldLabel("Version")
                Text(updates.currentVersion)
                    .font(TC.Font_.ledger)
                    .textSelection(.enabled)
            }

            switch updates.mode {
            case .selfUpdating:
                TCTag(text: "Checks daily", tone: .clear, symbol: "arrow.triangle.2.circlepath")
                Text(lastCheckSentence)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                // Deliberately does NOT claim the download already happened.
                // With SUAutomaticallyUpdate false, Sparkle's stock driver
                // finds the update in the background and then asks; the
                // download follows the yes. Copy that promised an
                // already-downloaded update would be describing a
                // configuration this app does not ship.
                Text("""
                    Trace Commons looks for new versions on its own. Nothing on \
                    disk changes until you say yes.
                    """)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Check Now") { updates.checkNow() }
                    .buttonStyle(.bordered)
                    .disabled(!updates.canCheckNow)

            case .managedByHomebrew(let command):
                TCTag(text: "Updates managed by Homebrew", tone: .held, symbol: "shippingbox")
                Text("""
                    Homebrew installed this copy, so Homebrew replaces it. Run \
                    this in a terminal:
                    """)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: TC.Space.s) {
                    Text(command)
                        .font(TC.Font_.ledger)
                        .textSelection(.enabled)
                        .padding(TC.Space.s)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .tcCard()
                    Button("Copy") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(command, forType: .string)
                    }
                    .buttonStyle(.bordered)
                }

            case .disabled(let reason):
                TCTag(text: "Updates unavailable", tone: .refused, symbol: "arrow.down.circle")
                Text(disabledSentence(reason))
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var lastCheckSentence: String {
        guard let date = updates.lastCheckDate else {
            return "Not checked yet on this machine."
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return "Last checked \(formatter.localizedString(for: date, relativeTo: Date()))."
    }

    /// Turns the policy's stable label into a sentence. The label itself is
    /// what gets logged; this is what a person reads.
    private func disabledSentence(_ reason: String) -> String {
        switch reason {
        case UpdatePolicy.noFeedReason:
            return """
                This build has no update feed configured, so it will not look \
                for new versions. Development builds are like this. Install \
                from a release DMG to receive updates.
                """
        case UpdatePolicy.insecureFeedReason:
            return """
                This build's update feed is not HTTPS, so it has been refused. \
                Reinstall from a release DMG.
                """
        default:
            return "Updates are turned off for this build."
        }
    }
```

Add the imports this section needs at the top of the file, changing `import SwiftUI` (line 1) to:

```swift
import AppKit
import SwiftUI
import TCUpdates
```

- [ ] **Step 3: Build**

```bash
cargo build -p trace-commons-contributor-ffi
swift build --package-path macos --arch arm64 --arch x86_64
```

Expected: `Build complete!`.

- [ ] **Step 4: Verify the Homebrew branch renders, by making the condition true**

The detector reads a real path, so the way to see this state is to create the path. Use a throwaway prefix under `/usr/local` — this creates one empty directory and removes it again.

```bash
cargo build -p trace-commons-contributor-ffi
./macos/scripts/make-app-bundle.sh debug 0.0.0-dev 1

sudo mkdir -p /usr/local/Caskroom/trace-commons
TRACE_COMMONS_SHOW_WINDOW=1 macos/.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp &
```

Open the Settings screen in the window that appears. Expected: an "Updates" section showing version `0.0.0-dev`, the pill "Updates managed by Homebrew", and the command `brew upgrade --cask trace-commons` in a copyable card with a Copy button. There must be no "Check Now" button.

Then clean up and confirm the other branch:

```bash
kill %1
sudo rmdir /usr/local/Caskroom/trace-commons
TRACE_COMMONS_SHOW_WINDOW=1 macos/.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp &
```

Expected: the same section now reads "Updates unavailable" with the no-feed sentence, because a dev bundle carries no `SUFeedURL` (Task 5). Then:

```bash
kill %1
sudo rmdir /usr/local/Caskroom 2>/dev/null || true
```

- [ ] **Step 5: Verify the self-updating branch renders**

A dev bundle has no feed, so force one for the render check only. This uses a URL that does not resolve; the point is the Settings state, not a successful fetch.

```bash
/usr/libexec/PlistBuddy -c \
  "Add :SUFeedURL string https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml" \
  macos/.build/TraceCommons.app/Contents/Info.plist
/usr/libexec/PlistBuddy -c "Add :SUPublicEDKey string dGVzdA==" \
  macos/.build/TraceCommons.app/Contents/Info.plist
codesign --force --sign - --timestamp=none macos/.build/TraceCommons.app
TRACE_COMMONS_SHOW_WINDOW=1 macos/.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp &
```

Expected in Settings: the "Checks daily" pill, "Not checked yet on this machine.", the sentence about downloading in the background, and an enabled "Check Now" button. Clicking it shows Sparkle's own alert (it will report a failure, since the test key cannot verify the real appcast — that failure is the fail-closed behavior working).

```bash
kill %1
rm -rf macos/.build/TraceCommons.app
```

- [ ] **Step 6: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Views/SettingsView.swift
git commit -m "Show update state and the Homebrew upgrade command in Settings"
```

---

## Verification

All of these must pass before this plan is complete. Run from the repo root.

```bash
# Prerequisite: swift test builds every target, including the ones that link
# the Rust FFI dylib.
cargo build -p trace-commons-contributor-ffi

# Swift unit tests: version comparison against the shared fixture, Homebrew
# detection, and the policy that gates Sparkle.
swift test --package-path macos --filter TCUpdatesTests

# The universal build, which is what the DMG ships.
swift build --package-path macos --arch arm64 --arch x86_64

# The shell-script contract: Info.plist keys, fail-closed behavior without a
# key, framework embedding, and inside-out signing order in both scripts.
cargo test -p trace-commons-contributor --test release_pipeline

# The repo-wide Rust gates, since this plan touches a Rust test file.
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check

# The assembled bundle. Must print "verified universal (arm64 x86_64):
# Sparkle framework" and then verify.
./macos/scripts/make-app-bundle.sh debug 0.0.0-dev 1
codesign --verify --deep --strict --verbose=2 macos/.build/TraceCommons.app
otool -L macos/.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp | grep -i sparkle
ls macos/.build/TraceCommons.app/Contents/Frameworks/Sparkle.framework/Versions/B

# Both Info.plist shapes are well-formed.
TC_SPARKLE_PUBLIC_ED_KEY=dGVzdA== ./macos/scripts/info-plist.sh 0.4.2 17 | plutil -lint -
./macos/scripts/info-plist.sh 0.4.2 17 | plutil -lint -

# No other dependency crept in.
cat macos/Package.resolved
```

Expected: every command exits 0; `otool -L` prints exactly one Sparkle line reading `@rpath/Sparkle.framework/Versions/B/Sparkle`; `ls` shows `Autoupdate`, `Modules`, `Resources`, `Sparkle`, `Updater.app`, `XPCServices`; `plutil -lint` prints `-: OK` twice; `Package.resolved` names `sparkle` at `2.9.6` and nothing else.

### Task 10: Publish the signed appcast from the release pipeline

**Files:**
- Modify: `.github/workflows/release-apps.yml` (extend the existing `publish-updates` job)
- Modify: `crates/trace-commons-contributor/tests/release_pipeline.rs` (append one test)

**Interfaces:**
- Consumes: `scripts/updates/generate-appcast.sh` (manifest-publishing plan, Task 4); `macos/.build/artifacts/sparkle/Sparkle/bin/sign_update` (Task 4 of this plan); the `macos-dmg` artifact; the `sparkle-signing-key` secret.
- Produces: `updates/appcast.xml` in the `tracecommons-flatpak` bucket — the feed `SUFeedURL` points at.

**Why this task exists and why it is here rather than in the other plan.**
The manifest-publishing plan writes `generate-appcast.sh` but deliberately never
runs it: `sign_update` ships inside Sparkle's SwiftPM artifact bundle and does
not exist on any runner until this plan adds Sparkle as a dependency. Both plans
originally assumed the other published the appcast, so nothing did — and an
unpublished appcast means `SUFeedURL` 404s and every update check fails closed
and silently, which is the hardest failure mode to notice. This task closes that
gap.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn the_release_workflow_publishes_the_appcast() {
    let wf = read(".github/workflows/release-apps.yml");
    assert!(
        wf.contains("generate-appcast.sh"),
        "release-apps.yml must run generate-appcast.sh. Without it SUFeedURL \
         404s and every Sparkle check fails closed and silently -- the app \
         reports no update forever and nothing logs an error."
    );
    assert!(
        wf.contains("sparkle-signing-key"),
        "the appcast must be signed with the Sparkle EdDSA key from Secret Manager"
    );
    assert!(
        wf.contains("appcast.xml"),
        "the generated appcast must be uploaded to the bucket"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline the_release_workflow_publishes_the_appcast`
Expected: FAIL — "release-apps.yml must run generate-appcast.sh".

- [ ] **Step 3: Extend the publish-updates job**

The `publish-updates` job already resolves a real OpenSSL, fetches the manifest
signing key, builds and verifies `latest.json`, and uploads it. Add the appcast
alongside, gated on the macOS build having succeeded — an appcast naming a DMG
that was never built would point every installed Mac at a 404.

Insert these steps into `publish-updates`, after the manifest verification step
and before the upload step:

```yaml
      # Sparkle's sign_update lives inside the SwiftPM artifact bundle, so the
      # package has to be resolved before the appcast can be signed. This is
      # why appcast publication lives in the Sparkle plan and not in the
      # manifest-publishing job that produces latest.json.
      - name: Resolve Sparkle so sign_update exists
        if: needs.macos.result == 'success'
        run: swift package --package-path macos resolve

      - name: Import the Sparkle signing key
        if: needs.macos.result == 'success'
        env:
          GCP_PROJECT: tracecommons-pilot-2026
        run: |
          set -euo pipefail
          gcloud secrets versions access latest \
            --secret=sparkle-signing-key --project "$GCP_PROJECT" \
            > "$RUNNER_TEMP/sparkle-ed-key"
          chmod 600 "$RUNNER_TEMP/sparkle-ed-key"

      - name: Generate the signed appcast
        if: needs.macos.result == 'success'
        env:
          SHORT_VERSION: ${{ needs.version.outputs.short }}
          BUILD_VERSION: ${{ needs.version.outputs.build }}
          REPO: ${{ github.repository }}
        run: |
          set -euo pipefail
          V="$SHORT_VERSION"
          SIGN_UPDATE="macos/.build/artifacts/sparkle/Sparkle/bin/sign_update"
          test -x "$SIGN_UPDATE" || {
            echo "sign_update missing at $SIGN_UPDATE" >&2; exit 1; }
          ./scripts/updates/generate-appcast.sh \
            --short-version "$V" \
            --build-version "$BUILD_VERSION" \
            --dmg-url "https://github.com/$REPO/releases/download/app-v$V/TraceCommons-$V.dmg" \
            --dmg-path "dist/macos-dmg/TraceCommons-$V.dmg" \
            --sign-update "$SIGN_UPDATE" \
            --out dist/updates/appcast.xml
          cat dist/updates/appcast.xml
```

Then change the existing upload step to include the appcast when it was built:

```yaml
      - name: Publish to the bucket
        env:
          BUCKET: tracecommons-flatpak
        run: |
          set -euo pipefail
          gcloud storage cp --cache-control="public, max-age=300" \
            dist/updates/latest.json dist/updates/latest.json.sig \
            "gs://$BUCKET/updates/"
          # Conditional on the file rather than on a job result, so a macOS
          # failure simply leaves the previous appcast in place rather than
          # replacing it with one pointing at a DMG that does not exist.
          if [ -f dist/updates/appcast.xml ]; then
            gcloud storage cp --cache-control="public, max-age=300" \
              dist/updates/appcast.xml "gs://$BUCKET/updates/"
          fi
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: PASS, all tests.

Then confirm the workflow still parses:

Run: `python3 -c "import yaml; d=yaml.safe_load(open('.github/workflows/release-apps.yml')); print(sorted(d['jobs'].keys()))"`
Expected: the same six job names as before, unchanged.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release-apps.yml \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Publish the signed Sparkle appcast when a macOS release is cut"
```

---

## Not verified by this plan

Two things remain unproven when every command above passes, and neither may be described as working:

1. **Developer ID signing and notarization of the Sparkle-bearing bundle.** `make-release-dmg.sh` has never been run against real credentials. The gate is a real run producing a signed, notarized, stapled DMG, followed by opening that DMG on a Mac that did not build it, with the network off, and confirming it launches with no Gatekeeper prompt.
2. **An end-to-end update.** That needs a published, EdDSA-signed `appcast.xml` in the bucket (the manifest-publishing plan) and two consecutive signed releases. Sparkle also requires the *installed* app's Developer ID signature to match the update's, so the first release that can ever be updated from is the first one signed with the production identity.

## Operator prerequisites

Not code; required before the first release that uses this plan.

1. Generate the Sparkle EdDSA keypair with `macos/.build/artifacts/sparkle/Sparkle/bin/generate_keys`. It stores the private key in the login keychain and prints the base64 public key.
2. Store the private key in GCP Secret Manager as `sparkle-signing-key` in `tracecommons-pilot-2026`, for `scripts/updates/generate-appcast.sh` to use.
3. Add the printed public key as the GitHub Actions secret `SPARKLE_PUBLIC_ED_KEY` in the `release` environment. It is a public key, but it is added as a secret so that a change to it is an audited action rather than an ordinary commit.
4. Confirm `https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml` is publicly readable once Task 10 of this plan uploads it. An app that cannot fetch its appcast fails closed and silently.
