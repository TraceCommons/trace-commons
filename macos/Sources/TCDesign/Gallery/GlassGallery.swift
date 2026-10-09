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
    @State private var select = "watch"
    @State private var radio = "quick"
    @State private var consent = false
    @State private var notes = ""
    @State private var secret = ""
    @State private var showModal = false
    @State private var showConfirm = false
    @State private var choice = "graph"

    private let scrolls: Bool
    private let scene: Bool

    /// `scrolls: false` lays the gallery out at full height, for snapshot
    /// renders (`ImageRenderer` does not draw a scroll view's content).
    /// `scene: false` draws no ground, for a glass window whose panes show
    /// the desktop (`TCDesignGallery`).
    public init(scrolls: Bool = true, scene: Bool = true) {
        self.scrolls = scrolls
        self.scene = scene
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
        .background(scene ? GlassTokens.Color.sceneBase.color : .clear)
        .glassModalHost()
    }

    private var sections: some View {
        VStack(alignment: .leading, spacing: 28) {
            section("Colour") { colour }
            section("Materials") { materials }
            section("Type") { type }
            section("Controls") { controls }
            section("Forms") { forms }
            section("Feedback") { feedback }
            section("Modal") { modal }
            section("Navigation") { navigation }
            section("Indicators") { indicators }
            section("Patterns") { patterns }
            section("Floating") { floatingLayer }
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
            .glassType(GlassTokens.TypeScale.body)
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
            ForEach(["display", "heading", "title", "body", "bodyStrong", "label", "caption", "micro", "eyebrow", "mono", "number"], id: \.self) { name in
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
                Button("Delete") {}.buttonStyle(GlassButtonStyle(.destructive))
            }
            // A set of glass buttons, one chosen (selected), as #1146's
            // aria-pressed.
            HStack(spacing: 6) {
                ForEach(["graph", "map", "list"], id: \.self) { name in
                    Button(name) { choice = name }.buttonStyle(GlassButtonStyle(.glass, selected: choice == name))
                }
            }
            HStack(spacing: 10) {
                GlassRoundButton("Settings", systemImage: "gearshape") {}
                GlassPillIconButton("previous", systemImage: "chevron.left") {}
                GlassToolbarGroup {
                    GlassToolbarButton("view", systemImage: "line.3.horizontal", expanded: true) {}
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
                Toggle("Toggle", isOn: $toggle).toggleStyle(GlassToggleStyle(showsLabel: false))
                Toggle("settings", isOn: $settingsToggle).toggleStyle(GlassToggleStyle(.settings, showsLabel: false))
                Toggle("watch", isOn: $watched).toggleStyle(GlassToggleStyle(.watch, showsLabel: false))
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

    private var forms: some View {
        HStack(alignment: .top, spacing: 24) {
            VStack(alignment: .leading, spacing: 12) {
                GlassTextField("Invalid", text: $field, invalid: true).frame(width: 220)
                GlassTextField("Passphrase", text: $secret, prompt: "prompt", secure: true).frame(width: 220)
                GlassTextField("Disabled", text: $field).frame(width: 220).disabled(true)
                GlassTextArea("Notes", text: $notes, prompt: "prompt").frame(width: 260)
            }
            VStack(alignment: .leading, spacing: 12) {
                GlassSelect(
                    "rule",
                    selection: $select,
                    options: [
                        GlassPickerOption("Watch", value: "watch", dot: .on),
                        GlassPickerOption("Ask", value: "ask", dot: .ask),
                    ]
                )
                GlassRadioGroup(
                    "mode",
                    selection: $radio,
                    options: [
                        GlassRadioOption("Quick", value: "quick", sub: "sub"),
                        GlassRadioOption("Custom", value: "custom"),
                        GlassRadioOption("Disabled", value: "disabled", isEnabled: false),
                    ]
                )
                GlassCheckRow("consent", isOn: $consent).frame(width: 220)
            }
            GlassCard(flush: true) {
                VStack(spacing: 0) {
                    GlassTableHead {
                        HStack {
                            Text("Name")
                            Spacer()
                            Text("Count")
                        }
                    }
                    GlassTableRow { Text("row").foregroundStyle(GlassColor.textPrimary) }
                    GlassTableRow { Text("row").foregroundStyle(GlassColor.textPrimary) }
                }
            }
            .frame(width: 240)
        }
    }

    private var feedback: some View {
        HStack(alignment: .center, spacing: 20) {
            GlassSpinner("Loading")
            GlassSpinner(size: 20)
            VStack(alignment: .leading, spacing: 6) {
                GlassSkeleton(width: 180)
                GlassSkeleton(width: 120)
            }
            GlassWarningGlyph("Warning").foregroundStyle(GlassTokens.Color.statusAskText.color)
            GlassWarningGlyph(size: 16).foregroundStyle(GlassTokens.Color.statusOutsideText.color)
            GlassCard(interactive: true) {
                Text("interactive").glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textTertiary)
            }
            .frame(width: 160)
        }
    }

    /// A modal drawn in place, then buttons that raise one over the whole
    /// gallery and a confirmation over that.
    private var modal: some View {
        HStack(alignment: .top, spacing: 20) {
            GlassModal(
                title: "Rename",
                subtitle: "subtitle",
                width: .narrow,
                closeLabel: "Close",
                actions: [
                    .cancel("Cancel") {},
                    GlassModalAction("Rename", isDefault: true) {},
                ],
                onCancel: {}
            ) {
                GlassTextField("Name", text: $field).padding(18)
            }
            // Drawn in place, it is not raised, so it takes no keys.
            .environment(\.glassModalIsTopmost, false)
            VStack(alignment: .leading, spacing: 10) {
                Button("Modal") { showModal = true }.buttonStyle(GlassButtonStyle(.glass))
                Button("Confirm") { showConfirm = true }.buttonStyle(GlassButtonStyle(.glass))
            }
        }
        .glassModal(isPresented: $showModal) {
            GlassModal(
                title: "Preview",
                closeLabel: "Close",
                actions: [
                    .cancel("Cancel") { showModal = false },
                    .destructive("Delete") { showConfirm = true },
                    GlassModalAction("Done", isDefault: true) { showModal = false },
                ],
                onCancel: { showModal = false }
            ) {
                GlassSkeleton().padding(18)
            }
            .glassModal(isPresented: $showConfirm) {
                GlassConfirmation(
                    title: "Delete",
                    message: "message",
                    actions: [
                        .cancel("Cancel") { showConfirm = false },
                        .destructive("Delete") { showConfirm = false; showModal = false },
                    ],
                    onCancel: { showConfirm = false }
                )
            }
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
                    GlassListRow(depth: .session, tile: .session, title: "trace", sub: "flagged", flag: .ask, selected: true, submitTitle: "Review", menuLabel: "menu", onSubmit: {}, onMenu: {})
                    GlassListRow(depth: .session, tile: .session, title: "trace", sub: "paused", off: true, submitTitle: "Submit", menuLabel: "menu", onMenu: {})
                }
            }
            .frame(width: 420, height: 190)
            VStack(alignment: .leading, spacing: 10) {
                GlassEyebrowCard("History", action: {}) {
                    Text("accessory").glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
                } content: {
                    Text("content").glassType(GlassTokens.TypeScale.bodyStrong).foregroundStyle(GlassColor.textPrimary)
                }
                GlassConsentBlock("consent")
                GlassKeyValueList([.init("Path", "~/code/orchard-api", mono: true), .init("Traces", "18")])
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

    /// The floating layer over a stand-in map field: Liquid Glass on macOS
    /// 26, the painted tiers before it.
    private var floatingLayer: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: GlassTokens.Radius.pane, style: .continuous)
                .fill(RadialGradient(
                    colors: [GlassTokens.Color.mapFieldInner.color, GlassTokens.Color.mapFieldOuter.color],
                    center: .center, startRadius: 10, endRadius: 420))
            ForEach(0..<14, id: \.self) { index in
                Circle()
                    .fill(index % 3 == 0 ? GlassTokens.Color.purpleSoft.color : GlassTokens.Color.blue.color)
                    .frame(width: 18, height: 18)
                    .offset(x: CGFloat(60 + (index * 97) % 620), y: CGFloat(70 + (index * 53) % 230))
            }
            GlassFloatingGroup {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 10) {
                        GlassSegmentedTabs(
                            "map",
                            selection: $mapTab,
                            segments: [
                                GlassSegment("Traces", value: "traces"),
                                GlassSegment("AI", value: "ai", dot: .on),
                            ],
                            floating: true
                        )
                        Spacer()
                        GlassToolbarGroup {
                            GlassToolbarButton("graph", systemImage: "chart.bar", pressed: true) {}
                            GlassToolbarButton("map", systemImage: "map") {}
                        }
                        GlassRoundButton("Settings", systemImage: "gearshape") {}
                    }
                    Spacer()
                    GlassNodeCard("nodeCard", detail: "detail", hint: "hint")
                        .frame(width: GlassTokens.Size.nodeCardWidth)
                }
                .padding(14)
            }
        }
        .frame(width: 720, height: 340)
    }
}

#Preview("GlassGallery") {
    GlassGallery()
}
#endif
