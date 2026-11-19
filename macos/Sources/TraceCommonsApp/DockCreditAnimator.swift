import AppKit
import TCDesign
import TCShellCore

/// Abstraction over `NSApp.dockTile` and `NSApp.requestUserAttention` so
/// `DockCreditAnimator` can be unit-tested without mutating the system Dock.
@MainActor
protocol DockTilePresenter: AnyObject {
    var tileSize: CGSize { get }
    var backingScale: CGFloat { get }
    func baseIcon() -> NSImage?
    func requestAttention()
    func show(_ image: NSImage)
    func restore()
}

@MainActor
private final class AppKitDockTilePresenter: DockTilePresenter {
    private var cachedBaseIcon: NSImage?
    private var imageView: NSImageView?

    var tileSize: CGSize {
        let size = NSApp.dockTile.size
        return (size.width > 0 && size.height > 0) ? size : GlassDockCoinBurst.defaultSize
    }

    var backingScale: CGFloat {
        NSScreen.main?.backingScaleFactor ?? 2
    }

    func baseIcon() -> NSImage? {
        if let cachedBaseIcon { return cachedBaseIcon }
        guard let icon = NSApp.applicationIconImage?.copy() as? NSImage else { return nil }
        cachedBaseIcon = icon
        return icon
    }

    func requestAttention() {
        NSApp.requestUserAttention(.informationalRequest)
    }

    func show(_ image: NSImage) {
        let size = tileSize
        let view = imageView ?? {
            let created = NSImageView(frame: NSRect(origin: .zero, size: size))
            created.imageScaling = .scaleProportionallyUpOrDown
            imageView = created
            return created
        }()
        if view.frame.size != size {
            view.frame = NSRect(origin: .zero, size: size)
        }
        view.image = image
        if NSApp.dockTile.contentView !== view {
            NSApp.dockTile.contentView = view
        }
        NSApp.dockTile.display()
    }

    func restore() {
        NSApp.dockTile.contentView = nil
        NSApp.dockTile.display()
    }
}

/// Animates the app's Dock icon with a gold coin burst (and a single
/// informational bounce when the app is in the background) whenever NEAR AI
/// credits arrive. Restores the native Dock icon when finished or on shutdown.
@MainActor
final class DockCreditAnimator {
    static let shared = DockCreditAnimator()

    private let presenter: any DockTilePresenter
    private let isEnabled: () -> Bool
    private let reduceMotion: () -> Bool
    private let sleep: (Duration) async -> Void
    private var task: Task<Void, Never>?

    init(
        presenter: any DockTilePresenter = AppKitDockTilePresenter(),
        isEnabled: @escaping () -> Bool = { BalanceArrival.IconAnimation.isEnabled() },
        reduceMotion: @escaping () -> Bool = { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion },
        sleep: @escaping (Duration) async -> Void = { duration in try? await Task.sleep(for: duration) }
    ) {
        self.presenter = presenter
        self.isEnabled = isEnabled
        self.reduceMotion = reduceMotion
        self.sleep = sleep
    }

    /// Runs one credit-arrival celebration on the Dock icon. Calling again
    /// while a celebration is in progress restarts cleanly from the start.
    func celebrate() {
        guard isEnabled() else {
            stop()
            return
        }
        task?.cancel()
        let still = reduceMotion()
        if !still {
            presenter.requestAttention()
        }
        guard let baseIcon = presenter.baseIcon() else {
            presenter.restore()
            return
        }
        let tileSize = presenter.tileSize
        let scale = presenter.backingScale
        let frames = BalanceArrival.IconAnimation.frameTimes(reduceMotion: still)
        let wait = still
            ? BalanceArrival.IconAnimation.reduceMotionHold
            : BalanceArrival.IconAnimation.frameInterval
        let presenter = self.presenter
        let sleep = self.sleep

        task = Task { [weak self] in
            for elapsed in frames {
                guard !Task.isCancelled else { return }
                let image = GlassDockCoinBurst.render(
                    baseIcon: baseIcon,
                    at: elapsed,
                    flash: !still,
                    tileSize: tileSize,
                    scale: scale
                )
                presenter.show(image)
                await sleep(wait)
            }
            guard !Task.isCancelled else { return }
            presenter.restore()
            self?.task = nil
        }
    }

    /// Cancels any active animation and immediately restores the system Dock icon.
    func stop() {
        task?.cancel()
        task = nil
        presenter.restore()
    }
}
