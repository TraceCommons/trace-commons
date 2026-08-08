import AppKit
import Foundation
import SwiftUI

/// Writes PNGs of the shell's real views, driven by the real running daemon,
/// when `TRACE_COMMONS_SCREENSHOT_DIR` is set.
///
/// A development hook, not a product feature. It exists because a menu-bar
/// app has to be *seen* to be verified, and it rasterizes with
/// `ImageRenderer` rather than photographing windows: `screencapture` and
/// `cacheDisplay` both come back blank when the desktop session is locked,
/// since nothing is being composited. `ImageRenderer` runs on the CPU and
/// does not care.
///
/// What it renders is the shipping view hierarchy bound to live daemon data
/// -- the same `MainWindowView`, `MenuBarContent` and `PreviewSheet` a person
/// sees -- not a mock-up. The one accommodation is that `ImageRenderer`
/// never runs `task`/`onAppear`, so the sheet is handed content that was
/// loaded first through the ordinary preview path.
enum DebugScreenshot {
    static var directory: String? {
        let value = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"]
        return (value?.isEmpty == false) ? value : nil
    }

    @MainActor
    static func scheduleIfRequested(model: AppModel) {
        guard let directory else { return }
        Task { @MainActor in
            // Late enough that the watcher has polled, queued, and scrubbed.
            try? await Task.sleep(nanoseconds: 12_000_000_000)

            render(
                QueueContent(previewing: .constant(nil)).environmentObject(model),
                to: directory + "/macos-shell-window.png",
                size: CGSize(width: 860, height: 640)
            )
            render(
                MenuBarPreview(model: model),
                to: directory + "/macos-shell-menu-bar.png",
                size: CGSize(width: 380, height: 330)
            )
            render(
                ConsentScopesContent().environmentObject(model),
                to: directory + "/macos-shell-consent-scopes.png",
                size: CGSize(width: 660, height: 760)
            )
            if let (entry, preloaded) = await model.loadCaptureSample(needle: "Northwind") {
                render(
                    PreviewSheet(entry: entry, preloaded: preloaded).environmentObject(model),
                    to: directory + "/macos-shell-preview-sheet.png",
                    size: CGSize(width: 820, height: 620)
                )
            }
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_QUIT_AFTER_SHOT"] == "1" {
                // Late enough that the self-test, which starts on the same
                // clock, has finished writing.
                try? await Task.sleep(nanoseconds: 25_000_000_000)
                model.shutdown()
                NSApp.terminate(nil)
            }
        }
    }

    @MainActor
    private static func render<V: View>(_ view: V, to path: String, size: CGSize) {
        let renderer = ImageRenderer(
            content: view
                .frame(width: size.width, height: size.height)
                .background(Color(nsColor: .windowBackgroundColor))
        )
        renderer.scale = 2
        guard let image = renderer.nsImage,
              let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let data = rep.representation(using: .png, properties: [:])
        else {
            NSLog("trace-commons: could not render \(path)")
            return
        }
        try? data.write(to: URL(fileURLWithPath: path))
        NSLog("trace-commons: wrote \(path)")
    }
}

/// The menu-bar item and its menu, side by side, so one image shows both the
/// badge and what the menu says. The menu itself is an AppKit-owned surface
/// and cannot be rasterized in place.
private struct MenuBarPreview: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 6) {
                Text("Menu bar:").font(.caption).foregroundStyle(.secondary)
                MenuBarLabel(model: model)
            }
            Divider()
            VStack(alignment: .leading, spacing: 6) {
                MenuBarContent()
                    .environmentObject(model)
            }
            .font(.callout)
        }
        .padding(16)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
