// swift-tools-version:6.0
import PackageDescription

// Milestone package: proves the trace-commons-contributor-ffi C ABI is
// callable from Swift. Links the Rust dylib built at
// ../target/debug/libtrace_commons_contributor_ffi.dylib (cargo build -p
// trace-commons-contributor-ffi must be run first, from the repo root).
let package = Package(
    name: "TraceCommonsFFIDemo",
    platforms: [.macOS(.v13)],
    targets: [
        .systemLibrary(
            name: "CTraceCommons"
        ),
        .executableTarget(
            name: "tc-ffi-demo",
            dependencies: ["CTraceCommons"],
            linkerSettings: [
                .unsafeFlags([
                    "-L", "../target/debug",
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
        ),
    ]
)
