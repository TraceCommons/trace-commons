import SwiftUI
import TCBridge

/// "Look inside": the one surface in the product that deliberately shows
/// trace content, because consent to send something you cannot see is not
/// consent.
///
/// Four tabs, in the spec's order. **Search is first and focused** on
/// purpose: "does this mention my client's name?" is a question a
/// contributor can answer in five seconds. Judging redaction quality by eye
/// is not, and this interface never asks them to.
struct PreviewSheet: View {
    /// Content already loaded elsewhere, so the sheet can be rendered
    /// without running its `task`. Used only by the screenshot hook, which
    /// has to rasterize the real view (`ImageRenderer` never runs `task` or
    /// `onAppear`) rather than photograph a window.
    struct Preloaded {
        let summary: PreviewSummary
        let transcript: String
        let needle: String
        let offsets: [Int]
    }

    let entry: QueueEntry
    let preloaded: Preloaded?

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var current: QueueEntry
    @State private var remaining: [QueueEntry] = []
    @State private var preview: TCPreview?
    @State private var summary: PreviewSummary?
    @State private var transcriptText: String
    @State private var failure: String?
    @State private var loading: Bool
    /// Search first, always: it is the question a contributor can actually
    /// answer in five seconds.
    @State private var tab: Tab = .search

    enum Tab: String, CaseIterable, Identifiable {
        case search, whatsInIt, transcript, permissions
        var id: String { rawValue }
        var title: String {
            switch self {
            case .search: return "Search"
            case .whatsInIt: return "What's in it"
            case .transcript: return "Exactly what would be sent"
            case .permissions: return "Permissions"
            }
        }

        var symbol: String {
            switch self {
            case .search: return "magnifyingglass"
            case .whatsInIt: return "list.bullet.rectangle"
            case .transcript: return "doc.plaintext"
            case .permissions: return "checklist"
            }
        }
    }

    init(entry: QueueEntry, preloaded: Preloaded? = nil) {
        self.entry = entry
        self.preloaded = preloaded
        _current = State(initialValue: entry)
        _summary = State(initialValue: preloaded?.summary)
        _transcriptText = State(initialValue: preloaded?.transcript ?? "")
        _loading = State(initialValue: preloaded == nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            content
            Divider()
            footer
        }
        .frame(width: 820, height: 620)
        .tcScreen()
        .task(id: current.entryID) {
            guard preloaded == nil else { return }
            await load()
        }
        .onAppear {
            remaining = model.awaitingDecision.filter { $0.entryID != current.entryID }
        }
        .onDisappear { closePreview() }
    }

    // MARK: - Chrome

    /// The same identity line and the same labelled figures as a queue card,
    /// in the same order. Recognising the card you just clicked is one of
    /// the quieter things that makes a preview trustworthy.
    private var header: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                Text(current.projectLabel).font(TC.Font_.cardTitle)
                Text(current.agentName)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(.secondary)
                Spacer(minLength: TC.Space.m)
                Text(Format.when(current.discoveredAt))
                    .font(TC.Font_.footnote)
                    .foregroundStyle(.tertiary)
            }
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.xxl) {
                if let summary {
                    VStack(alignment: .leading, spacing: TC.Space.xxs) {
                        TCFieldLabel("Would send")
                        Text(Format.bytes(summary.wouldSendBytes))
                            .font(TC.Font_.ledger)
                            .monospacedDigit()
                    }
                    .accessibilityElement(children: .combine)
                }
                VStack(alignment: .leading, spacing: TC.Space.xxs) {
                    TCFieldLabel("Status")
                    TCTag(text: "nothing sent yet", tone: .clear, symbol: "lock")
                }
                .accessibilityElement(children: .combine)
                Spacer(minLength: 0)
            }
            Text("Nothing has been sent. This is what would be.")
                .font(TC.Font_.footnote)
                .foregroundStyle(.secondary)
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TC.surface)
    }

    @ViewBuilder
    private var content: some View {
        if loading {
            CenteredNotice(
                title: "Scrubbing it locally…",
                detail: "Reading the session and running the redaction pass."
            )
        } else if let failure {
            CenteredNotice(
                title: "This one can't be shown.",
                detail: """
                \(failure). Nothing has been sent, and nothing will be until it can \
                be shown to you.
                """
            )
        } else if let summary {
            // A segmented control rather than a TabView: inside a sheet this
            // is the standard macOS treatment, and Search has to be able to
            // start selected and focused.
            //
            // It is built from Buttons rather than `Picker(.segmented)`
            // because each segment needs to carry a glyph AND a count -- the
            // number of things scrubbing removed sits on "What's in it", so
            // a person can see there is something to look at before they
            // click the tab. A stock segmented picker takes labels only.
            VStack(alignment: .leading, spacing: TC.Space.m) {
                tabBar(summary)

                switch tab {
                case .search:
                    SearchTab(
                        transcript: transcriptText,
                        preview: preview,
                        initialNeedle: preloaded?.needle ?? "",
                        initialOffsets: preloaded?.offsets
                    )
                case .whatsInIt:
                    WhatsInItTab(entry: current, summary: summary)
                case .transcript:
                    TranscriptTab(transcript: transcriptText)
                case .permissions:
                    PermissionsTab(summary: summary, options: model.consentScopes)
                }
                Spacer(minLength: 0)
            }
            .padding(TC.Space.m)
        }
    }

    /// The four tabs, in the spec's order, each a plain button. The one that
    /// has something to report says so on its face.
    private func tabBar(_ summary: PreviewSummary) -> some View {
        HStack(spacing: TC.Space.xxs) {
            ForEach(Tab.allCases) { item in
                Button {
                    tab = item
                } label: {
                    HStack(spacing: TC.Space.xs) {
                        Image(systemName: item.symbol)
                            .imageScale(.small)
                        Text(item.title)
                        if let note = badge(for: item, summary: summary) {
                            Text(note)
                                .font(TC.Font_.ledger)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .font(TC.Font_.footnote.weight(tab == item ? .bold : .regular))
                    .foregroundStyle(tab == item ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                    .padding(.horizontal, TC.Space.m)
                    .padding(.vertical, TC.Space.xs)
                    .background {
                        RoundedRectangle(cornerRadius: TC.Radius.inset)
                            .fill(tab == item ? TC.surface : Color.clear)
                    }
                    .overlay {
                        RoundedRectangle(cornerRadius: TC.Radius.inset)
                            .strokeBorder(
                                tab == item ? TC.green.opacity(0.55) : Color.clear,
                                lineWidth: TC.Space.hairline
                            )
                    }
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(tab == item ? [.isSelected, .isButton] : .isButton)
            }
            Spacer(minLength: 0)
        }
        .padding(TC.Space.xxs)
        .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.card))
    }

    private func badge(for item: Tab, summary: PreviewSummary) -> String? {
        switch item {
        case .whatsInIt:
            let removed = summary.redactions.values.reduce(0, +)
            return removed == 0 ? nil : "\(removed)"
        case .permissions:
            return "\(summary.consentScopes.count)"
        default:
            return nil
        }
    }

    /// The one irreversible click in the product, and the one place the
    /// scrubbing caveat is repeated verbatim on purpose -- see
    /// `ScrubbingCaveat`.
    private var footer: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            ScrubbingCaveatAtCommit()
            HStack(spacing: TC.Space.s) {
                Button("Not this one") {
                    model.dismiss(current)
                    advance()
                }
                // Untinted: it must not read as a second way to approve.
                .tint(.primary)
                if !remaining.isEmpty {
                    Text("\(remaining.count) more after this")
                        .font(TC.Font_.footnote)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: TC.Space.m)
                Button("Close") { dismiss() }
                // The ONLY approve control in the product, and it is behind
                // the preview by design.
                Button("Contribute") {
                    model.approve(current)
                    advance()
                }
                .tcPrimaryAction()
                .keyboardShortcut(.defaultAction)
                .disabled(summary == nil)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TC.surface)
    }

    // MARK: - Flow

    /// Advancing to the next entry inside the sheet is what makes three
    /// sessions three deliberate clicks in one flow. There is no select-all.
    private func advance() {
        closePreview()
        guard let next = remaining.first else {
            dismiss()
            return
        }
        remaining.removeFirst()
        summary = nil
        transcriptText = ""
        failure = nil
        loading = true
        current = next
    }

    private func load() async {
        loading = true
        let outcome = await model.openPreview(entryID: current.entryID)
        switch outcome {
        case .opened(let opened):
            preview = opened
            transcriptText = opened.body
            if let data = opened.summaryJSON.data(using: .utf8),
               let decoded = try? DaemonDecoding.decoder().decode(PreviewSummary.self, from: data)
            {
                summary = decoded
            } else {
                failure = "the summary could not be read"
            }
        case .failed(let message):
            failure = message
        }
        loading = false
    }

    private func closePreview() {
        preview?.close()
        preview = nil
    }
}

// MARK: - Tabs

/// The highest-value affordance in the product: type a client name, get
/// `0 matches` or jump-to-context, without reading 148 turns.
struct SearchTab: View {
    let transcript: String
    let preview: TCPreview?

    @State private var needle: String
    @State private var offsets: [Int]?
    @State private var searched: Bool
    @State private var recents: [String] = RecentSearches.load()
    @FocusState private var focused: Bool

    init(
        transcript: String,
        preview: TCPreview?,
        initialNeedle: String = "",
        initialOffsets: [Int]? = nil
    ) {
        self.transcript = transcript
        self.preview = preview
        _needle = State(initialValue: initialNeedle)
        _offsets = State(initialValue: initialOffsets)
        _searched = State(initialValue: initialOffsets != nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Search this trace for anything you need to be sure isn't in it.")
                .font(.callout)
            TextField("Client name, hostname, anything", text: $needle)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit(run)
                .onChange(of: needle) { _, _ in run() }

            if !recents.isEmpty {
                HStack(spacing: 6) {
                    Text("Recent:").font(.caption).foregroundStyle(.secondary)
                    ForEach(recents, id: \.self) { term in
                        Button(term) { needle = term }
                            .buttonStyle(.link)
                            .font(.caption)
                    }
                }
            }

            resultSummary

            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(contexts.enumerated()), id: \.offset) { _, snippet in
                        Text(snippet)
                            .font(.system(.callout, design: .monospaced))
                            .textSelection(.enabled)
                            .padding(8)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
                    }
                }
            }
        }
        .onAppear { focused = true }
    }

    @ViewBuilder
    private var resultSummary: some View {
        if !searched || needle.isEmpty {
            Text("Type to search. Nothing is sent while you look.")
                .font(.callout)
                .foregroundStyle(.secondary)
        } else if offsets == nil {
            Text("The search couldn't run on this trace.")
                .font(.callout)
                .foregroundStyle(.secondary)
        } else if offsets!.isEmpty {
            // The answer to the only question this tab exists for, in
            // the app's two loudest tones -- each with a glyph, because a
            // green word and an amber word are the same word in greyscale.
            Label("0 matches", systemImage: TC.Tone.clear.symbol)
                .font(TC.Font_.sectionTitle)
                .foregroundStyle(TC.Tone.clear.textColor)
        } else {
            Label("^[\(offsets!.count) match](inflect: true)", systemImage: TC.Tone.attention.symbol)
                .font(TC.Font_.sectionTitle)
                .foregroundStyle(TC.Tone.attention.textColor)
        }
    }

    /// Runs on the main actor deliberately: the scan is a local in-memory
    /// pass, and keeping every touch of the `tc_preview*` on one thread is
    /// what the header's ownership rules ask for -- its pointer check narrows
    /// accidental misuse to an error, it does not make concurrent use safe.
    private func run() {
        searched = true
        guard !needle.isEmpty, let preview else {
            offsets = []
            return
        }
        offsets = preview.search(needle)
        if let offsets, !offsets.isEmpty {
            recents = RecentSearches.remember(needle)
        }
    }

    /// The ABI reports UTF-8 BYTE offsets, so context is cut from the byte
    /// array and decoded back, never from Swift's character indices.
    private var contexts: [String] {
        guard let offsets, !offsets.isEmpty else { return [] }
        let bytes = Array(transcript.utf8)
        let window = 120
        return offsets.prefix(20).map { offset in
            let start = max(0, offset - window)
            let end = min(bytes.count, offset + needle.utf8.count + window)
            guard start < end else { return "" }
            let slice = Array(bytes[start..<end])
            let text = String(decoding: slice, as: UTF8.self)
                .replacingOccurrences(of: "\n", with: " ")
            return (start > 0 ? "…" : "") + text + (end < bytes.count ? "…" : "")
        }
    }
}

struct WhatsInItTab: View {
    let entry: QueueEntry
    let summary: PreviewSummary

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                LabeledContent("Agent", value: entry.agentName)
                LabeledContent("Project", value: entry.projectLabel)
                LabeledContent("Turns recorded", value: "\(summary.eventCount)")
                LabeledContent("Session on disk", value: Format.bytes(summary.rawSessionBytes))
                LabeledContent("Would send", value: Format.bytes(summary.wouldSendBytes))
                Text("""
                "Would send" is usually larger than the file on disk: a redacted \
                envelope also carries schema, consent and privacy metadata the raw \
                session file does not.
                """)
                .font(.caption)
                .foregroundStyle(.secondary)

                Divider()
                Text("What scrubbing removed").font(.headline)
                if summary.redactions.isEmpty {
                    Text("""
                    Nothing matched. On a session that touched credentials, that is \
                    itself worth a second look.
                    """)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                } else {
                    ForEach(summary.redactions.sorted(by: { $0.key < $1.key }), id: \.key) { kind, count in
                        Text("\(count) × \(kind.replacingOccurrences(of: "_", with: " "))")
                            .font(.callout)
                    }
                }

                if !summary.piiLabelsPresent.isEmpty {
                    Divider()
                    Text("Personal-information categories seen").font(.headline)
                    Text(summary.piiLabelsPresent.joined(separator: ", "))
                        .font(.callout)
                    Text("Categories only. The matched text is never reported here.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Divider()
                Text("Residual risk: \(summary.residualRisk.replacingOccurrences(of: "_", with: " "))")
                    .font(.callout)
                Text("""
                Files touched and tools invoked are not in this contract's preview \
                summary, so they are not shown rather than guessed at.
                """)
                .font(.caption)
                .foregroundStyle(.tertiary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(4)
        }
    }
}

/// The redacted transcript exactly as it would be sent. Redactions stay
/// visible as inline chips (`[SECRET]`, `[PATH]`) rather than deletions, so a
/// contributor can see WHERE scrubbing fired -- which is the point.
struct TranscriptTab: View {
    let transcript: String

    var body: some View {
        ScrollView([.vertical, .horizontal]) {
            Text(transcript.isEmpty ? "(empty)" : transcript)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(6)
        }
    }
}

/// The consent scopes this upload will carry, restated at the moment of
/// consent rather than only at onboarding.
struct PermissionsTab: View {
    let summary: PreviewSummary
    let options: [ConsentScope]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("What this upload asks for").font(.headline)
                ForEach(summary.consentScopes, id: \.self) { scope in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(ScopeCopy.title(for: scope, options: options))
                            .font(.callout.weight(.semibold))
                        if let description = options.first(where: { $0.name == scope })?.description {
                            Text(description).font(.callout).foregroundStyle(.secondary)
                        }
                    }
                }
                Text("""
                These are the permissions this device requests. Trace Commons can \
                narrow them, never widen them -- and if your permissions change \
                between now and sending, this approval stops applying and you are \
                asked again.
                """)
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(4)
        }
    }
}

enum ScopeCopy {
    /// The first words of each label carry the distinction, because that is
    /// all most people read.
    static func title(for wireName: String, options: [ConsentScope]) -> String {
        switch wireName {
        case "debugging_evaluation": return "Finding bugs and measuring agents"
        case "benchmark_only", "benchmark_creation": return "Turn my traces into test cases"
        case "ranking_training", "reward_model_training":
            return "Train models that judge agent output"
        case "model_training": return "Train coding models directly"
        case "public_attribution": return "List my handle publicly as a contributor"
        default:
            return options.first(where: { $0.name == wireName })?.name
                ?? wireName.replacingOccurrences(of: "_", with: " ")
        }
    }
}

enum RecentSearches {
    private static let key = "trace-commons.recent-searches"

    static func load() -> [String] {
        UserDefaults.standard.stringArray(forKey: key) ?? []
    }

    /// Recent searches persist so the second trace is one keystroke. They
    /// are the contributor's own words, kept locally, and never sent.
    static func remember(_ term: String) -> [String] {
        var terms = load().filter { $0 != term }
        terms.insert(term, at: 0)
        terms = Array(terms.prefix(6))
        UserDefaults.standard.set(terms, forKey: key)
        return terms
    }
}
