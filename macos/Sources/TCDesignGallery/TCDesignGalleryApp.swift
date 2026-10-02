import SwiftUI
import TCDesign

/// The gallery in a floating glass window: the panes show the desktop
/// through Liquid Glass (macOS 26) or the HUD material (14–25), and the gap
/// between them is the desktop itself.
@main
struct TCDesignGalleryApp: App {
    init() {
        NSApplication.shared.setActivationPolicy(.regular)
    }

    var body: some Scene {
        WindowGroup {
            // The gallery is debug-only, so a release build of this tool
            // opens an empty glass window.
            #if DEBUG
            GlassGallery(scene: false)
                .glassWindow()
                .onAppear { NSApp.activate(ignoringOtherApps: true) }
            #else
            Color.clear.glassWindow()
            #endif
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1320, height: 900)
    }
}
