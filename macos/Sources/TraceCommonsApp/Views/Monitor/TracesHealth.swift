#if DEBUG
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What the Traces tab says about the core's health, fail closed: a core that
/// does not answer, a status nobody could read and every reported condition
/// is a banner; only a status that was read and has nothing wrong draws none.
/// Every word is the core's.
@MainActor
enum TracesHealth {
    struct Banner: Equatable, Identifiable {
        let title: String
        let detail: String?
        let tone: GlassStatus
        var id: String { title }
    }

    /// The core-down line as the production tab passes it.
    static let coreDownLine = HealthLineCopy.decode(
        fromJSON: TCCoreCopy.healthCopyJSON(reachable: false, label: nil, maxQueueEntries: nil))

    static func banners(
        phase: TracesStore.Phase, status: DaemonData.Status?, words: MonitorTracesCopy?, coreDown: HealthLineCopy?
    ) -> [Banner] {
        let drawn: [Banner]
        switch phase {
        case .failed(.unreachable):
            if let coreDown {
                drawn = [Banner(title: coreDown.title, detail: coreDown.detail, tone: .outside)]
            } else {
                drawn = [Banner(title: words?.line(for: .unreachable) ?? "", detail: nil, tone: .outside)]
            }
        case .failed(let other):
            drawn = [Banner(title: words?.line(for: other) ?? "", detail: nil, tone: .outside)]
        case .loading where status == nil:
            drawn = []
        case .loaded where status == nil:
            // The tab loaded but the status read did not: unknown, not healthy.
            drawn = [Banner(title: words?.requestFailed ?? "", detail: nil, tone: .outside)]
        default:
            // A waiting line is still not healthy, so it is `.ask`; `.off`
            // would read as "off".
            drawn = TracesStore.safeguards(status).map {
                Banner(title: $0.title, detail: $0.body, tone: .ask)
            }
        }
        return drawn.filter { !$0.title.isEmpty }
    }
}

/// One health banner. It carries no action: the glass tree has no level for
/// the legacy queue-root button to return to, and an absent control reads
/// better than a disabled one.
struct GlassHealthBanner: View {
    let banner: TracesHealth.Banner

    var body: some View {
        GlassNotice(tone: banner.tone, title: banner.title) {
            if let detail = banner.detail { Text(detail) }
        }
        .accessibilityElement(children: .combine)
    }
}
#endif
