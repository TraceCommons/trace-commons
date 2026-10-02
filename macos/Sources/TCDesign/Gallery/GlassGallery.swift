#if DEBUG
import SwiftUI

/// Every component of the glass design system, live. Debug builds only:
/// the shipping app does not carry it. The C2 contract's
/// preview gallery: open it in Xcode's canvas, or show it from a debug
/// build. Placeholders are single words and token names on purpose: this
/// shell authors no sentences (ShellWordingTests), and real wording reaches
/// the screens from the Rust core.
public struct GlassGallery: View {
    @State private var tab = "home"
    @State private var mapTab = "traces"
    @State private var toggle = true
    @State private var settingsToggle = false
    @State private var watched = true
    @State private var checked = true
    @State private var children = [true, false]
    @State private var rule: String? = "watch"
    @State private var expanded = true
    @State private var field = "8463"
    @State private var hovered: String?

    private let scrolls: Bool

    /// `scrolls: false` lays the gallery out at full height, for snapshot
    /// renders (`ImageRenderer` does not draw a scroll view's content).
    public init(scrolls: Bool = true) {
        self.scrolls = scrolls
    }

    public var body: some View {
        Group {
            if scrolls {
                ScrollView { sections }
            } else {
                sections
            }
        }
        .frame(minWidth: 900, minHeight: 700)
        .background(GlassTokens.Color.sceneBase.color)
        .preferredColorScheme(.dark)
    }

    private var sections: some View {
        VStack(alignment: .leading, spacing: 28) {
            section("Colour") { colour }
            section("Materials") { materials }
            section("Type") { type }
            section("Controls") { controls }
            section("Navigation") { navigation }
            section("Indicators") { indicators }
            section("Patterns") { patterns }
        }
        .padding(28)
    }

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            GlassSectionRule(title)
            content()
        }
    }

    private var colour: some View {
        HStack(alignment: .top, spacing: 24) {
            HStack(spacing: 10) {
                ForEach(["purple", "purpleSoft", "purpleText", "blue"], id: \.self) { name in
                    VStack(spacing: 4) {
                        RoundedRectangle(cornerRadius: 10).fill(GlassTokens.Color.all[name]?.color ?? .clear).frame(width: 52, height: 36)
                        Text(name).glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textTertiary)
                    }
                }
            }
            VStack(alignment: .leading, spacing: 6) {
                GlassStatusLabel("statusOn", status: .on)
                GlassStatusLabel("statusAsk", status: .ask)
                GlassStatusLabel("statusOff", status: .off)
                GlassStatusLabel("statusOutside", status: .outside)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("textPrimary").foregroundStyle(GlassColor.textPrimary)
                Text("textSecondary").foregroundStyle(GlassColor.textSecondary)
                Text("textTertiary").foregroundStyle(GlassColor.textTertiary)
                Text("purpleText").foregroundStyle(GlassColor.accentText)
            }
            .font(.system(size: 13))
        }
    }

    private var materials: some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.paneGap) {
            GlassPane {
                VStack(alignment: .leading, spacing: 10) {
                    Text("pane").glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textTertiary)
                    GlassCard {
                        HStack {
                            GlassWell { Color.clear.frame(height: 26) }
                            GlassRoundButton("Settings", systemImage: "gearshape") {}
                        }
                    }
                    GlassCard(quiet: true) {
                        Text("cardQuiet").glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textTertiary)
                    }
                }
            }
            .frame(width: 320, height: 200)
            GlassNodeCard("nodeCard", detail: "detail", hint: "hint")
            GlassMenu {
                GlassMenuItem("checked", checked: true) {}
                GlassMenuSeparator()
                GlassMenuItem("disabled") {}.disabled(true)
            }
            .frame(width: 240)
        }
    }

    private var type: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(["display", "heading", "title", "body", "bodyStrong", "label", "caption", "eyebrow", "mono", "number"], id: \.self) { name in
                if let style = GlassTokens.TypeScale.all[name] {
                    Text(name)
                        .glassType(style)
                        .foregroundStyle(GlassColor.textPrimary)
                }
            }
        }
    }

    private var controls: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Button("Primary") {}.buttonStyle(GlassButtonStyle(.primary))
                Button("Disabled") {}.buttonStyle(GlassButtonStyle(.primary)).disabled(true)
                Button("Secondary") {}.buttonStyle(GlassButtonStyle(.secondary))
                Button("Glass") {}.buttonStyle(GlassButtonStyle(.glass))
                Button("Submit·3") {}.buttonStyle(GlassButtonStyle(.submit(done: false)))
                Button("Sent") {}.buttonStyle(GlassButtonStyle(.submit(done: true)))
                Button("Link") {}.buttonStyle(GlassButtonStyle(.link))
            }
            HStack(spacing: 10) {
                GlassRoundButton("Settings", systemImage: "gearshape") {}
                GlassPillIconButton("previous", systemImage: "chevron.left") {}
                GlassToolbarGroup {
                    GlassToolbarButton("view", systemImage: "line.3.horizontal") {}
                    GlassToolbarButton("graph", systemImage: "chart.bar", pressed: true) {}
                    GlassToolbarButton("map", systemImage: "map", pressed: false) {}
                    GlassToolbarButton("inspector", systemImage: "sidebar.right", pressed: true) {}
                }
                GlassFolderButton("folder") {}
                GlassKebab("menu") {}
                GlassPicker(
                    "rule",
                    selection: $rule,
                    options: [
                        GlassPickerOption("Watch", value: "watch", dot: .on),
                        GlassPickerOption("Ask", value: "ask", dot: .ask),
                        GlassPickerOption("Never", value: "never", dot: .off),
                    ],
                    placeholder: "rule"
                )
            }
            HStack(spacing: 14) {
                Toggle("Toggle", isOn: $toggle).labelsHidden().toggleStyle(GlassToggleStyle())
                Toggle("settings", isOn: $settingsToggle).labelsHidden().toggleStyle(GlassToggleStyle(.settings))
                Toggle("watch", isOn: $watched).labelsHidden().toggleStyle(GlassToggleStyle(.watch))
                Toggle("Single", isOn: $checked).toggleStyle(GlassCheckboxStyle())
                Toggle("Group", sources: $children, isOn: \.self)
                    .toggleStyle(GlassCheckboxStyle())
                GlassCheckMark(checked: true)
                GlassCheckMark(checked: false)
                GlassExpander("Decisions", isOpen: $expanded).frame(width: 140)
            }
            GlassTextField("Port", text: $field).frame(width: 220)
        }
    }

    private var navigation: some View {
        VStack(alignment: .leading, spacing: 14) {
            GlassSegmentedTabs(
                "Monitor",
                selection: $tab,
                segments: [
                    GlassSegment("Home", value: "home"),
                    GlassSegment("Inference", value: "inference", dot: .on),
                    GlassSegment("Traces", value: "traces", badge: 9),
                ]
            )
            .frame(width: 360)
            GlassSegmentedTabs(
                "map",
                selection: $mapTab,
                segments: [
                    GlassSegment("Traces", value: "traces"),
                    GlassSegment("AI", value: "ai", dot: .on),
                ],
                floating: true
            )
            GlassBreadcrumb([GlassCrumb("Home") {}, GlassCrumb("Missions")], backLabel: "Home") {}
            GlassStepProgress(labels: ["Folders", "Join", "Uses", "Inference", "Sharing", "Projects"], current: 2)
                .frame(width: 520)
        }
    }

    private var indicators: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                GlassChip("Scrubbed", status: .on)
                GlassChip("Flagged", status: .ask)
                GlassChip("Outside", status: .outside)
                GlassTag("Contributed", tone: .on)
                GlassTag("Review", tone: .ask)
                GlassTag("Neutral")
                GlassTag("Drafts", tone: .accent)
                GlassBadge(count: 14)
                GlassBadge(count: 9, subtle: true)
            }
            HStack(spacing: 8) {
                GlassToolTile(.tool(.claudeCode))
                GlassToolTile(.tool(.codex))
                GlassToolTile(.tool(.antigravity))
                GlassToolTile(.tool(.openCode))
                GlassToolTile(.tool(.theia))
                GlassToolTile(.folder)
                GlassToolTile(.session)
                GlassStatusDot(.on, ring: true)
                GlassStatusDot(.shared, halo: true)
            }
            // Large tiles, then the tools with no artwork yet, which fall
            // back to their initials.
            HStack(spacing: 8) {
                GlassToolTile(.tool(.claudeCode), large: true)
                GlassToolTile(.tool(.codex), large: true)
                GlassToolTile(.tool(.antigravity), large: true)
                GlassToolTile(.tool(.openCode), large: true)
                GlassToolTile(.tool(.theia), large: true)
                GlassToolTile(.tool(.geminiCLI), large: true)
                GlassToolTile(.tool(.cline), large: true)
            }
            HStack(spacing: 6) {
                GlassLegendCell("shared", value: "5", status: .shared)
                GlassLegendCell("kept", value: "7", status: .kept)
            }
            .frame(width: 300)
            GlassBarGraph(
                zip(["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"], [(3.0, 2.0), (1, 4), (0, 1), (4, 0), (2, 3), (0, 1), (5, 2)])
                    .map { GlassBarBucket(id: $0.0, label: $0.0, up: $0.1.0, down: $0.1.1, description: $0.0) },
                scaleFloor: 5,
                hovered: $hovered
            )
            .frame(width: 380)
        }
    }

    private var patterns: some View {
        HStack(alignment: .top, spacing: 20) {
            GlassPane(padding: 8) {
                VStack(spacing: 2) {
                    GlassListRow(depth: .tool, tile: .tool(.claudeCode), title: "tool", sub: "sub", expanded: true, submitTitle: "Submit·3", watched: $watched, onToggleExpand: {}, onSubmit: {})
                    GlassListRow(depth: .folder, tile: .folder, title: "folder", sub: "sub", expanded: true, submitTitle: "Submit·3", watched: $watched, menuLabel: "menu", onToggleExpand: {}, onSubmit: {}, onMenu: {})
                    GlassListRow(depth: .session, tile: .session, title: "session", sub: "flagged", flag: .ask, selected: true, submitTitle: "Review", menuLabel: "menu", onSubmit: {}, onMenu: {})
                }
            }
            .frame(width: 420, height: 150)
            VStack(alignment: .leading, spacing: 10) {
                GlassEyebrowCard("History", action: {}) {
                    Text("accessory").font(.system(size: 11)).foregroundStyle(GlassColor.textSecondary)
                } content: {
                    Text("content").font(.system(size: 13, weight: .semibold)).foregroundStyle(GlassColor.textPrimary)
                }
                GlassConsentBlock("consent")
                GlassKeyValueList([.init("Path", "~/code/orchard-api", mono: true), .init("Sessions", "18")])
                GlassNotice(tone: .ask, title: "notice") {
                    Text("body")
                }
                GlassCard(flush: true) {
                    VStack(spacing: 0) {
                        GlassTableRow(first: true) { Text("row").foregroundStyle(GlassColor.textPrimary) }
                        GlassTableRow { Text("row").foregroundStyle(GlassColor.textPrimary) }
                    }
                }
            }
            .frame(width: 360)
        }
    }
}

#Preview("GlassGallery") {
    GlassGallery()
}
#endif
