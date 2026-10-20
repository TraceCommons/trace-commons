// swift-tools-version: 6.0
// The macOS menu bar app — a pure client of the IronWire control API.
//
// SwiftPM rather than an `.xcodeproj`: the release job has to build this from a
// script, and a package manifest is a file a reviewer can actually read. Xcode
// opens `Package.swift` directly, so nothing is lost. `macos/README.md` has the
// full reasoning.

import PackageDescription

let package = Package(
    name: "IronWire",
    platforms: [
        // `MenuBarExtra` is the whole app and it arrived in Ventura.
        .macOS(.v13)
    ],
    targets: [
        // Everything that can be decided without a window. The app target is
        // views and wiring only, so the rules that matter — no bar for an
        // unknown headroom, no notification below the cross-family rung — are
        // testable without launching anything.
        .target(name: "IronWireKit", path: "Sources/IronWireKit"),
        .executableTarget(
            name: "IronWire",
            dependencies: ["IronWireKit"],
            path: "Sources/IronWire"
        ),
        .testTarget(
            name: "IronWireKitTests",
            dependencies: ["IronWireKit"],
            path: "Tests/IronWireKitTests"
        ),
        // The dropdown, rendered off-screen. Several states it has to get right
        // are ones a real daemon will not produce on request, and the most
        // important of them is an absence.
        .testTarget(
            name: "IronWireAppTests",
            dependencies: ["IronWire", "IronWireKit"],
            path: "Tests/IronWireAppTests"
        ),
    ]
)
