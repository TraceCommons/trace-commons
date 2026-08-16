// swift-tools-version:6.0
import PackageDescription
import Foundation

// The macOS contributor shell, plus the milestone demo that proves the
// trace-commons-contributor-ffi C ABI is callable from Swift. Both link the
// Rust dylib from make-app-bundle.sh's TC_FFI_LIB_DIR, which defaults to
// ../target/debug/libtrace_commons_contributor_ffi.dylib for development
// (cargo build -p trace-commons-contributor-ffi must be run first, from the
// repo root).

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
