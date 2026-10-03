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
        /// Position plus title, so two lines with one title never collide.
        let id: String

        init(title: String, detail: String?, tone: GlassStatus, index: Int = 0) {
            self.title = title
            self.detail = detail
            self.tone = tone
            id = "\(index)-\(title)"
        }
    }

    /// The core's unknown word, the last resort for a down or unread state
    /// when the health words themselves cannot be read: a banner is never
    /// blank, and an absent signal never reads as healthy.
    static let unknownWord = MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())?.unknown

    /// The core-down line as the production tab passes it.
    static let coreDownLine = HealthLineCopy.decode(
        fromJSON: TCCoreCopy.healthCopyJSON(reachable: false, label: nil, maxQueueEntries: nil))

    static func banners(
        phase: TracesStore.Phase, status: DaemonData.Status?, words: MonitorTracesCopy?, coreDown: HealthLineCopy?
    ) -> [Banner] {
        func failed(_ title: String?, _ detail: String? = nil) -> [Banner] {
            if let title, !title.isEmpty { return [Banner(title: title, detail: detail, tone: .outside)] }
            guard let unknown = Self.unknownWord, !unknown.isEmpty else { return [] }
            return [Banner(title: unknown, detail: nil, tone: .outside)]
        }
        switch phase {
        case .failed(.unreachable):
            if let coreDown { return failed(coreDown.title, coreDown.detail) }
            return failed(words?.line(for: .unreachable))
        case .failed(let other):
            return failed(words?.line(for: other))
        case .loading where status == nil:
            return []
        case .loaded where status == nil:
            // The tab loaded but the status read did not: unknown, not healthy.
            return failed(words?.requestFailed)
        default:
            // A waiting line is still not healthy, so it is `.ask`; `.off`
            // would read as "off". A safeguard line with no title is dropped.
            return TracesStore.safeguards(status)
                .filter { !$0.title.isEmpty }
                .enumerated()
                .map { Banner(title: $1.title, detail: $1.body, tone: .ask, index: $0) }
        }
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
