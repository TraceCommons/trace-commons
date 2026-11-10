import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Loads the published terms through the core (`tc_terms_document_json`).
/// No bundled copy exists to fall back on: a terms document that cannot be
/// loaded cannot be accepted (spec, anti-goals), so `failed` offers only a
/// retry.
@MainActor
final class TermsLoader: ObservableObject {
    enum State: Equatable {
        case loading
        case loaded(FetchedTerms)
        /// The core's refusal label. A label, never shown as text.
        case failed(String)
    }

    @Published private(set) var state: State = .loading
    private let fetch: () async -> Result<FetchedTerms, TCTermsRefusal>
    private var generation = 0

    init(fetch: @escaping () async -> Result<FetchedTerms, TCTermsRefusal> = TCTerms.fetchDetached) {
        self.fetch = fetch
    }

    /// The document now loaded, or nil.
    var terms: FetchedTerms? {
        if case .loaded(let terms) = state { return terms }
        return nil
    }

    /// Fetch again from the start. A load that finishes after a newer one
    /// began is dropped, so the reader always shows the latest answer.
    func load() async {
        generation += 1
        let mine = generation
        state = .loading
        let result = await fetch()
        guard mine == generation else { return }
        switch result {
        case .success(let terms): state = .loaded(terms)
        case .failure(let refusal): state = .failed(refusal.label)
        }
    }

    /// Accepting what is loaded now, or nil while loading or failed.
    func acceptance(at date: Date) -> AcceptedTerms? {
        terms.map { AcceptedTerms($0, at: date) }
    }
}

enum TermsReaderLayout {
    /// The scroll region's proposed height.
    static let viewport: CGFloat = 260
    static let width: CGFloat = 420
}

/// The terms, drawn natively from the published document, in a fixed-height
/// scroll region; the accept checkbox below unlocks once the end has been in
/// view (`TermsScrollGate`), and the Privacy Policy link sits under it.
/// Used by Welcome and by the new-terms sheet.
///
/// `accepted` is the caller's current acceptance; ticking hands
/// `AcceptedTerms` for the document on screen to `onAccepted`, unticking
/// hands nil. An acceptance of a version or hash other than the one now
/// loaded is withdrawn (handed nil) as soon as the reader sees it.
struct TermsReader: View {
    let copy: FirstRunCopy.Welcome.Legal
    @ObservedObject var loader: TermsLoader
    let accepted: AcceptedTerms?
    let onAccepted: (AcceptedTerms?) -> Void
    /// Whether the reader starts its own first load. The sheet loads on
    /// opening instead, so it fetches afresh every time.
    let autoload: Bool

    @State private var gate = TermsScrollGate()
    @AccessibilityFocusState private var lastBlockFocused: Bool
    private let space = "terms-reader"

    init(
        copy: FirstRunCopy.Welcome.Legal, loader: TermsLoader, accepted: AcceptedTerms?,
        autoload: Bool = true, onAccepted: @escaping (AcceptedTerms?) -> Void
    ) {
        self.copy = copy
        self.loader = loader
        self.accepted = accepted
        self.onAccepted = onAccepted
        self.autoload = autoload
    }

    var body: some View {
        VStack(spacing: GlassTokens.Space.s4) {
            region
                .frame(height: TermsReaderLayout.viewport)
                .background(
                    RoundedRectangle(cornerRadius: GlassTokens.Radius.control)
                        .fill(GlassColor.ink(0.04)))
                .overlay(
                    RoundedRectangle(cornerRadius: GlassTokens.Radius.control)
                        .strokeBorder(GlassColor.hairline))
            if let terms = loader.terms {
                if !terms.document.inEffect {
                    Text(copy.notInEffect)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                checkbox(terms)
            }
            Link(copy.privacy, destination: copy.privacyUrl)
                .glassType(GlassTokens.TypeScale.caption)
        }
        .frame(maxWidth: TermsReaderLayout.width)
        .task {
            if autoload, case .loading = loader.state { await loader.load() }
        }
        .onChange(of: loader.state) { _, state in
            // A new load starts reading from the top. Not on `.loaded`: a
            // document that fits reports its frame once, possibly before this
            // runs, and resetting then would never unlock it. Every load
            // passes through `.loading` first.
            if case .loaded = state {} else { gate = TermsScrollGate() }
            // A different document than the one accepted: ask again.
            if let accepted, case .loaded(let terms) = state,
                accepted.version != terms.document.version
                    || accepted.contentSHA256 != terms.contentSHA256
            {
                onAccepted(nil)
            }
        }
        .onChange(of: lastBlockFocused) { _, focused in
            if focused { gate.reachedLastBlock() }
        }
    }

    @ViewBuilder private var region: some View {
        switch loader.state {
        case .loading:
            VStack(spacing: GlassTokens.Space.s4) {
                ProgressView().controlSize(.small)
                Text(copy.loading)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .failed:
            VStack(spacing: GlassTokens.Space.s4) {
                Text(copy.failed)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                Button(copy.retry) { Task { await loader.load() } }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
            }
            .padding(GlassTokens.Space.s8)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .loaded(let terms):
            document(terms.document)
        }
    }

    private func document(_ document: TermsDocument) -> some View {
        GeometryReader { viewport in
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                    Text(document.title)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(Array(document.blocks.enumerated()), id: \.offset) { index, block in
                        // VoiceOver reaching the last block counts as the
                        // end. Not `onAppear`: a non-lazy stack makes every
                        // block appear at once.
                        if index == document.blocks.count - 1 {
                            TermsBlockView(block: block).accessibilityFocused($lastBlockFocused)
                        } else {
                            TermsBlockView(block: block)
                        }
                    }
                }
                .padding(GlassTokens.Space.s8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    GeometryReader { content in
                        Color.clear.preference(
                            key: TermsContentFrameKey.self, value: content.frame(in: .named(space)))
                    })
                .background(TermsScrollPositionProbe { content, viewport, offset in
                    gate.observe(contentHeight: content, viewportHeight: viewport, offset: offset)
                })
            }
            .modifier(TermsScrollGeometryWatch { content, viewport, offset in
                gate.observe(contentHeight: content, viewportHeight: viewport, offset: offset)
            })
            .coordinateSpace(name: space)
            // The first layout only: it settles a document that fits without
            // scrolling. Scrolling an NSScrollView does not lay its content
            // out again, so this never sees a scroll; the two watchers above
            // do (found 2026-11-09: the box never unlocked).
            .onPreferenceChange(TermsContentFrameKey.self) { frame in
                gate.observe(
                    contentHeight: frame.height, viewportHeight: viewport.size.height,
                    offset: -frame.minY)
            }
        }
    }

    private func checkbox(_ terms: FetchedTerms) -> some View {
        let isOn = Binding<Bool>(
            get: {
                accepted?.version == terms.document.version
                    && accepted?.contentSHA256 == terms.contentSHA256
            },
            set: { on in onAccepted(on ? AcceptedTerms(terms, at: Date()) : nil) })
        // Already accepted (Back to Welcome): the box stays usable to untick.
        let unlocked = gate.reachedEnd || isOn.wrappedValue
        return VStack(spacing: GlassTokens.Space.s2) {
            Toggle(copy.accept, isOn: isOn)
                .toggleStyle(GlassCheckboxStyle())
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .disabled(!unlocked)
                .help(unlocked ? "" : copy.scrollHint)
                .accessibilityHint(unlocked ? "" : copy.scrollHint)
            if !unlocked {
                Text(copy.scrollHint)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .accessibilityHidden(true)
            }
        }
    }
}

private struct TermsContentFrameKey: PreferenceKey {
    static let defaultValue: CGRect = .zero
    static func reduce(value: inout CGRect, nextValue: () -> CGRect) {
        value = nextValue()
    }
}

/// One block of the terms. Inline text is the document's limited Markdown
/// (emphasis, strong, code spans, links); links open in the browser.
struct TermsBlockView: View {
    let block: TermsDocument.Block

    var body: some View {
        switch block {
        case .heading(let text):
            inline(text)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .accessibilityAddTraits(.isHeader)
        case .paragraph(let text):
            inline(text).modifier(TermsBodyText())
        case .list(let items):
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                        Text(verbatim: "\u{2022}").accessibilityHidden(true)
                        inline(item)
                    }
                    .modifier(TermsBodyText())
                }
            }
        case .table(let header, let rows):
            Grid(alignment: .leading, horizontalSpacing: GlassTokens.Space.s6, verticalSpacing: GlassTokens.Space.s3) {
                GridRow {
                    ForEach(Array(header.enumerated()), id: \.offset) { _, cell in
                        inline(cell)
                            .glassType(GlassTokens.TypeScale.label)
                            .foregroundStyle(GlassColor.textPrimary)
                    }
                }
                Divider()
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    GridRow {
                        ForEach(Array(row.enumerated()), id: \.offset) { _, cell in
                            inline(cell).modifier(TermsBodyText())
                        }
                    }
                }
            }
        case .callout(let text):
            inline(text)
                .modifier(TermsBodyText())
                .padding(GlassTokens.Space.s6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: GlassTokens.Radius.tile)
                        .fill(GlassColor.ink(0.06)))
        }
    }

    private func inline(_ text: String) -> Text {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .inlineOnlyPreservingWhitespace)
        if let attributed = try? AttributedString(markdown: text, options: options) {
            return Text(attributed)
        }
        return Text(verbatim: text)
    }
}

private struct TermsBodyText: ViewModifier {
    func body(content: Content) -> some View {
        content
            .glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
    }
}

/// The scroll position as the scroll view itself reports it (macOS 15 and
/// later): content height, visible height, and the visible top.
private struct TermsScrollGeometryWatch: ViewModifier {
    let observe: (CGFloat, CGFloat, CGFloat) -> Void

    func body(content: Content) -> some View {
        if #available(macOS 15, *) {
            content.onScrollGeometryChange(for: CGRect.self) { geometry in
                CGRect(
                    x: 0, y: geometry.visibleRect.minY,
                    width: geometry.contentSize.height, height: geometry.visibleRect.height)
            } action: { _, seen in
                observe(seen.width, seen.height, seen.minY)
            }
        } else {
            content
        }
    }
}

/// The same reading on macOS 14, which has no scroll geometry callback:
/// the enclosing NSScrollView's clip view posts a bounds change on every
/// scroll. On macOS 15 and later it attaches nothing.
private struct TermsScrollPositionProbe: NSViewRepresentable {
    let observe: (CGFloat, CGFloat, CGFloat) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(observe: observe) }

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        if #unavailable(macOS 15) {
            let coordinator = context.coordinator
            // Not in a scroll view until it is in the window's hierarchy.
            DispatchQueue.main.async { MainActor.assumeIsolated { coordinator.attach(to: view) } }
        }
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.observe = observe
    }

    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        coordinator.detach()
    }

    @MainActor
    final class Coordinator {
        var observe: (CGFloat, CGFloat, CGFloat) -> Void
        private weak var scrollView: NSScrollView?
        private var token: NSObjectProtocol?

        init(observe: @escaping (CGFloat, CGFloat, CGFloat) -> Void) {
            self.observe = observe
        }

        func attach(to view: NSView) {
            guard token == nil, let scrollView = view.enclosingScrollView else { return }
            self.scrollView = scrollView
            scrollView.contentView.postsBoundsChangedNotifications = true
            token = NotificationCenter.default.addObserver(
                forName: NSView.boundsDidChangeNotification, object: scrollView.contentView, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.report() }
            }
            report()
        }

        func detach() {
            if let token { NotificationCenter.default.removeObserver(token) }
            token = nil
        }

        private func report() {
            guard let scrollView, let document = scrollView.documentView else { return }
            let visible = scrollView.contentView.documentVisibleRect
            let height = document.frame.height
            // Distance from the content's top to the visible top, whichever
            // way the document view is flipped.
            let offset = document.isFlipped ? visible.minY : height - visible.maxY
            observe(height, visible.height, offset)
        }
    }
}
