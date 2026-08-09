// swift-tools-version:6.0
import PackageDescription

// The macOS contributor shell, plus the milestone demo that proves the
// trace-commons-contributor-ffi C ABI is callable from Swift. Both link the
// Rust dylib built at ../target/debug/libtrace_commons_contributor_ffi.dylib
// (cargo build -p trace-commons-contributor-ffi must be run first, from the
// repo root).
let package = Package(
    name: "TraceCommons",
    platforms: [.macOS(.v14)],
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
        .executableTarget(
            name: "TraceCommonsApp",
            dependencies: ["TCBridge"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags([
                    "-L", "../target/debug",
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
                    "-L", "../target/debug",
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
        ),
    ]
)
