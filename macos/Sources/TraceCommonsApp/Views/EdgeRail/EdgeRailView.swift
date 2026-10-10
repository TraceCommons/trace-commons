import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The edge rail's content (`EdgeRailController`): closed, a short handle
/// on the screen edge; open, the icon rail with the hovered icon's peek
/// beside it. Every word is the core's (`MonitorEdgeRailCopy`, and the
/// tables each peek already shows elsewhere in the app).
struct EdgeRailView: View {
    let state: EdgeRailState
    /// Asks the controller to resize the panel for the new state.
    let onOpenChange: (Bool) -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The pending close: leaving the rail for a moment, as the pointer
    /// does crossing from an icon to its peek, does not close it.
    @State private var closing: Task<Void, Never>?

    private var rail: MonitorEdgeRailCopy? { MonitorWords.table?.edgeRail }

    var body: some View {
        Group {
            if state.open {
                HStack(alignment: .center, spacing: GlassTokens.Space.edgeRailGap) {
                    EdgeRailPeekCard(peek: state.peek, close: { setOpen(false) })
                        .frame(width: GlassTokens.Size.edgeRailPeekWidth)
                    icons
                }
                .padding(.trailing, GlassTokens.Space.edgeRailInset)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
                .transition(reduceMotion ? .opacity : .move(edge: .trailing).combined(with: .opacity))
            } else {
                handle
            }
        }
        .onHover { inside in
            if inside {
                closing?.cancel()
                closing = nil
                if !state.open { setOpen(true) }
            } else if state.open {
                closing = Task { @MainActor in
                    try? await Task.sleep(for: .milliseconds(300))
                    guard !Task.isCancelled else { return }
                    setOpen(false)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(rail?.railLabel ?? "")
    }

    private func setOpen(_ open: Bool) {
        // The panel grows before the rail draws into it, and shrinks after
        // the rail has gone.
        if open { onOpenChange(true) }
        withAnimation(reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide)) {
            state.open = open
        }
        if !open { onOpenChange(false) }
    }

    /// The closed rail: its handle on the edge (`GlassRailHandle`).
    private var handle: some View {
        GlassRailHandle()
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
            .contentShape(Rectangle())
            .accessibilityElement()
            .accessibilityLabel(rail?.handleLabel ?? "")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { setOpen(true) }
    }

    /// The icon rail, a pane on the desktop: one tile per peek, a rule,
    /// then the app.
    private var icons: some View {
        VStack(spacing: GlassTokens.Space.s1) {
            ForEach(EdgeRailPeek.allCases) { peek in
                EdgeRailTile(
                    systemImage: peek.symbol,
                    label: peek.name(rail, nav: MonitorWords.table?.settingsNav),
                    selected: state.peek == peek
                ) { state.peek = peek }
            }
            GlassRailRule()
            GlassRailTile(selected: false) {
                setOpen(false)
                OpenMonitor.request(nil)
            } label: {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: GlassTokens.Size.edgeRailTile - GlassTokens.Space.s7,
                           height: GlassTokens.Size.edgeRailTile - GlassTokens.Space.s7)
            }
            .help(rail?.openApp ?? "")
            .accessibilityLabel(rail?.openApp ?? "")
        }
        .padding(.vertical, GlassTokens.Space.s3)
        .frame(width: GlassTokens.Size.edgeRailWidth)
        .glassTier(.pane)
    }
}

/// One icon on the rail (`GlassRailTile`). Hovering it shows its peek, as
/// clicking does.
private struct EdgeRailTile: View {
    let systemImage: String
    let label: String
    let selected: Bool
    let select: () -> Void

    var body: some View {
        GlassRailTile(selected: selected, action: select) {
            Image(systemName: systemImage).glassGlyph(17, weight: .regular)
        }
        .onHover { if $0 { select() } }
        .help(label)
        .accessibilityLabel(label)
    }
}

/// The hovered icon's peek: a short summary of its part of the app, at
/// most one action, and a link into the app.
private struct EdgeRailPeekCard: View {
    let peek: EdgeRailPeek
    let close: () -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(ComputeModel.self) private var compute

    private var rail: MonitorEdgeRailCopy? { MonitorWords.table?.edgeRail }
    private var shell: MonitorShellCopy? { MonitorWords.table?.shell }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(peek == .privacy ? rail?.privacyTitle ?? "" : peek.name(rail, nav: MonitorWords.table?.settingsNav))
                .glassType(GlassTokens.TypeScale.heading)
                .foregroundStyle(GlassColor.textPrimary)
                .accessibilityAddTraits(.isHeader)
            content
            if !(peek == .waiting && model.awaitingDecision.count > 0) { link }
        }
        .padding(GlassTokens.Space.s9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassTier(.pane)
    }

    @ViewBuilder
    private var content: some View {
        switch peek {
        case .waiting: waiting
        case .tools: tools
        case .balance: balance
        case .privateAI: privateAI
        case .compute: computeSummary
        case .privacy: privacy
        }
    }

    // MARK: Peeks

    private var waiting: some View {
        let count = model.awaitingDecision.count
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(shell?.waiting(count, secondLook: 0) ?? "")
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            if count > 0 {
                Button(rail?.review ?? "") { open(.traces(entryId: nil)) }
                    .buttonStyle(GlassButtonStyle(.primary, small: true))
            }
        }
    }

    @ViewBuilder
    private var tools: some View {
        let rows = model.harnesses.harnesses
        if rows.isEmpty {
            caption(rail?.toolsEmpty ?? "")
        } else if let copy = model.privateInferenceCopy {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(rows) { row in
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                        Text(row.name)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        Spacer(minLength: 0)
                        Text(row.connected ? copy.harnessCaptionConnected : copy.harnessCaptionNotConnected)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .lineLimit(1)
                    }
                    .padding(.vertical, 9)
                    .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
                }
            }
        }
    }

    @ViewBuilder
    private var balance: some View {
        if let copy = model.privateInferenceCopy {
            BalanceRow(copy: copy)
        }
    }

    @ViewBuilder
    private var privateAI: some View {
        if let copy = model.privateInferenceCopy {
            let state = model.privateInferenceState
            let calls = model.privateInferenceCalls
            GlassStatusLabel(
                PrivateInferenceSurface.stateLine(state, copy: copy, calls: calls),
                status: PrivateInferenceIndicator.status(PrivateInferenceSurface.tone(state, calls: calls)))
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder
    private var computeSummary: some View {
        if let snapshot = compute.snapshot {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(snapshot.title)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                caption(snapshot.detail)
            }
        } else {
            caption(rail?.computeUnreported ?? "")
        }
    }

    private var privacy: some View {
        let paused = model.status.paused
        return VStack(alignment: .leading, spacing: 0) {
            privacyRow(rail?.onThisMac ?? "", status: .kept,
                       line: rail?.onThisMac(count: model.pending.count) ?? "")
            privacyRow(rail?.inTheLibrary ?? "", status: .shared,
                       line: rail?.inTheLibrary(count: model.history.count) ?? "")
            HStack(spacing: GlassTokens.Space.s3) {
                GlassStatusLabel(paused ? shell?.watcherPaused ?? "" : shell?.watcherWatching ?? "",
                                 status: paused ? .off : .on)
                Spacer(minLength: 0)
                Button(paused ? MenuBarWords.resume : MenuBarWords.pause) {
                    if paused { model.resume() } else { model.pause(until: nil) }
                }
                .buttonStyle(GlassButtonStyle(.glass, small: true))
            }
            .padding(.top, GlassTokens.Space.s4)
            .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
        }
    }

    private func privacyRow(_ title: String, status: GlassStatus, line: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            GlassStatusLabel(title, status: status)
                .frame(width: GlassTokens.Size.edgeRailPeekWidth / 3, alignment: .leading)
            Text(line)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 9)
        .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
    }

    // MARK: Shared

    /// The peek's link: the section's name, except Waiting, which opens the
    /// Traces tab by that tab's name. Waiting with sessions to review has
    /// its Review button instead, which goes to the same place.
    private var link: some View {
        let section = peek == .waiting ? shell?.tabTraces ?? "" : peek.name(rail, nav: MonitorWords.table?.settingsNav)
        let words = rail?.open(section: section)
        return HStack {
            Button { open(peek.destination) } label: {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(words?.text ?? "")
                    Image(systemName: "arrow.right").glassGlyph(10, weight: .semibold)
                }
            }
            .buttonStyle(GlassButtonStyle(.link))
            .accessibilityLabel(words?.label ?? "")
            Spacer(minLength: 0)
            Text(rail?.nothingSent ?? "")
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
        }
    }

    private func caption(_ sentence: String) -> some View {
        Text(sentence)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// Opens the app at the peek's destination and puts the rail away.
    private func open(_ destination: MonitorDestination) {
        close()
        OpenMonitor.request(destination)
    }
}
