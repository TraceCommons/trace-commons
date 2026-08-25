// swift-tools-version:6.0
import PackageDescription
import Foundation

// The macOS contributor shell, plus the milestone demo that proves the
// trace-commons-contributor-ffi C ABI is callable from Swift. Both link the
// Rust dylib from make-app-bundle.sh's TC_FFI_LIB_DIR, which defaults to
// ../target/debug for development (cargo build -p trace-commons-contributor-ffi
// must be run first, from the repo root; the dylib is found via -ltrace_commons_contributor_ffi).


// Which cargo profile's dylib to link against. make-app-bundle.sh exports
// this as <repo>/target/<config>; the default keeps a bare `swift build`
// working for development, which is the only reason the debug path is
// mentioned at all. It used to be hardcoded in both linker flags blocks,
// which meant `swift build -c release` linked against target/debug -- and
// failed outright on a CI checkout that never built debug.
let ffiLibDir = ProcessInfo.processInfo.environment["TC_FFI_LIB_DIR"]
    .flatMap { $0.isEmpty ? nil : $0 } ?? "../target/debug"

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
        .systemLibrary(
            name: "CTraceCommons"
        ),
        // The ONLY target that touches raw pointers.
        .target(
            name: "TCBridge",
            dependencies: ["CTraceCommons"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
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
        // State-directory resolution and the shell's refusal rules. Carved
        // out for the same reason TCUpdates was: keep logic that needs
        // neither the FFI dylib nor AppKit in a target that can be tested
        // without them. The bug this target exists to prevent -- resolving
        // the state directory from an environment variable a Finder launch
        // never has -- shipped in a notarized build precisely because there
        // was nowhere to test it.
        //
        // This is still the first place new logic should go. It is a
        // preference, not a limit: TraceCommonsAppTests (below) does link
        // the dylib and test the app target directly, for the things that
        // only exist there.
        .target(
            name: "TCShellCore",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "TCShellCoreTests",
            dependencies: ["TCShellCore"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .executableTarget(
            name: "TraceCommonsApp",
            dependencies: [
                "TCBridge",
                "TCShellCore",
                "TCUpdates",
                .product(name: "Sparkle", package: "Sparkle"),
            ],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags([
                    "-L", ffiLibDir,
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
        ),
        // The one test target that links the dylib. Everything testable
        // without it lives in TCShellCore and is tested there -- but "every
        // detector the scrubber actually has is labelled" cannot be asserted
        // against a fixture, only against the real table, and a list of what
        // is removed that quietly falls back to a de-slugged name is exactly
        // the drift this screen exists to avoid.
        .testTarget(
            name: "TCBridgeTests",
            dependencies: ["TCBridge", "TCShellCore"],
            swiftSettings: [.swiftLanguageMode(.v6)],
            linkerSettings: [
                .unsafeFlags([
                    "-L", ffiLibDir,
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
        ),
        // The app target itself. It links the FFI dylib and Sparkle, so
        // these are not unit tests in the sense TCShellCore's are -- prefer
        // that target for anything that does not need `AppModel`.
        //
        // What needs to live here is the observable behaviour of the model
        // the SwiftUI views actually bind to. The Done button shipped
        // broken because `markOnboardingComplete` mutated state without
        // publishing it, and no target existed that could observe an
        // `AppModel` and notice.
        .testTarget(
            name: "TraceCommonsAppTests",
            dependencies: ["TraceCommonsApp"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags([
                    "-L", ffiLibDir,
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
        ),
        .executableTarget(
            name: "tc-ffi-demo",
            dependencies: ["TCBridge"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags([
                    "-L", ffiLibDir,
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
        ),
    ]
)
