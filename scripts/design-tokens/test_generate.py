"""Safety contracts for the solid Linux translation of the flat theme."""
import json
import unittest

import generate


class InsightsTokensTests(unittest.TestCase):
    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def test_text_contrast_on_every_solid_surface(self):
        for scheme in generate.SCHEMES:
            p = generate.insights_palette(self.tokens, scheme)
            for ink in ('ink', 'secondary'):
                for surface in ('ground', 'card', 'hover', 'control', 'control_hover'):
                    with self.subTest(scheme=scheme, ink=ink, surface=surface):
                        self.assertGreaterEqual(generate.contrast(p[ink], p[surface]), 4.5)
            self.assertGreaterEqual(generate.contrast(p['selected_ink'], p['selected']), 4.5)

    def test_unreadable_future_token_is_rejected(self):
        self.tokens['themes']['flatDark']['color']['textSecondary'] = {'hex': '#171b2d'}
        with self.assertRaisesRegex(generate.TokenError, 'secondary'):
            generate.insights_palette(self.tokens, 'dark')

    def test_nonuniform_fill_is_not_silently_flattened(self):
        self.tokens['themes']['flatDark']['gradient']['cardFill']['stops'][1]['hex'] = '#ffffff'
        with self.assertRaisesRegex(generate.TokenError, 'uniform'):
            generate.insights_palette(self.tokens, 'dark')

    def test_hover_is_visible_against_its_actual_surface(self):
        for scheme in generate.SCHEMES:
            p = generate.insights_palette(self.tokens, scheme)
            for hover, surface in [('hover', 'card'), ('control_hover', 'control')]:
                with self.subTest(scheme=scheme, hover=hover):
                    self.assertGreaterEqual(generate.contrast(p[hover], p[surface]), 1.15)
            css = generate.render_insights_css(self.tokens, scheme)
            self.assertIn(f"stackswitcher button:hover {{ background: {p['control_hover']}; }}", css)

    def test_invisible_future_hover_is_rejected(self):
        self.tokens['themes']['flatLight']['color']['controlOnCardHover']['alpha'] = 0
        with self.assertRaisesRegex(generate.TokenError, 'hover'):
            generate.insights_palette(self.tokens, 'light')

    def test_light_fallback_resolves_before_compositing(self):
        # The general opaque ground keeps its classic Light value.
        self.assertEqual(generate.insights_entry(self.tokens, 'color', 'paneOpaque', 'light')['hex'], '#f2f3f6')
        # Only Settings adopts the greyer ground, and the card fill is Light-only.
        self.assertEqual(generate.insights_entry(self.tokens, 'color', 'settingsGround', 'light')['hex'], '#e9ebef')
        self.assertEqual(generate.insights_entry(self.tokens, 'color', 'controlOnCard', 'light')['hex'], '#8a93a5')
        self.assertNotIn('controlOnCard', self.tokens['themes']['flatDark']['color'])
        self.assertNotIn('controlOnCardHover', self.tokens['themes']['flatDark']['color'])
        self.assertEqual(generate.opaque_over({'hex': '#ffffff', 'alpha': 0.5}, '#000000'), '#808080')

    def test_card_pill_inks_on_actual_flat_grounds(self):
        # Exercise the JSON contrast contracts on hosts without SwiftUI.
        for scheme in generate.SCHEMES:
            color = lambda name: generate.insights_entry(self.tokens, 'color', name, scheme)
            fill = lambda name: generate.insights_entry(self.tokens, 'gradient', name, scheme)['stops'][0]
            desktop = '#333333' if scheme == 'light' else '#ffffff'
            pane = generate.opaque_over(fill('paneFill'), generate.opaque_over(color('glassVeil'), desktop))
            for ground in [pane, color('paneOpaque')['hex'], color('settingsGround')['hex']]:
                for card_name in ['cardFill', 'cardFillQuiet']:
                    card = generate.opaque_over(fill(card_name), ground)
                    layers = ([color('controlOnCard'), color('controlOnCardHover')] if scheme == 'light'
                              else [fill('controlFill'), color('controlHover')])
                    for layer in layers:
                        pill = generate.opaque_over(layer, card)
                        for ink in ['textPrimary', 'textSecondary', 'destructiveText', 'statusOnText']:
                            with self.subTest(scheme=scheme, ground=ground, card=card_name, ink=ink):
                                self.assertGreaterEqual(generate.contrast(generate.opaque_over(color(ink), pill), pill), 4.5)
                        # statusOff is disabled submit text: measure the actual
                        # dimmed group, without claiming an active-text WCAG floor.
                        dim = self.tokens['opacity']['disabledSubmit']
                        disabled = generate.opaque_over(color('statusOff'), pill)
                        self.assertGreater(generate.contrast(
                            generate.opaque_over({'hex': disabled, 'alpha': dim}, card),
                            generate.opaque_over({'hex': pill, 'alpha': dim}, card)), 1)

    def test_type_roles_follow_desktop_font_size(self):
        for scheme in generate.SCHEMES:
            css = generate.render_insights_css(self.tokens, scheme)
            self.assertNotRegex(css, r"font-size: [\d.]+px")
            self.assertIn(".tc-insights { font-size: 1em;", css)
            self.assertIn(".tc-insights .title-3 { font-size: 2em;", css)
            self.assertIn(".tc-insights .caption { font-size: 0.846154em;", css)
        self.tokens['type']['body']['size'] = 14
        css = generate.render_insights_css(self.tokens, 'light')
        self.assertIn(".tc-insights .title-3 { font-size: 1.85714em;", css)

    def test_generated_rules_are_scoped_to_insights(self):
        for scheme in generate.SCHEMES:
            css = generate.render_insights_css(self.tokens, scheme)
            # Every rule is Insights', focused or in an unfocused glass window.
            self.assertTrue(all(line.startswith(('.tc-insights', 'window.tc-glass-window:backdrop .tc-insights'))
                                for line in css.splitlines()))
            self.assertIn('button:focus-visible', css)
            self.assertIn('button:checked', css)


class GlassTokensTests(unittest.TestCase):
    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def test_every_tier_has_an_opaque_fill_and_a_class(self):
        for scheme in generate.SCHEMES:
            palette = generate.glass_palette(self.tokens, scheme)
            css = generate.render_glass_css(self.tokens, scheme)
            for tier, name, _, _, _ in generate.GLASS_TIERS:
                with self.subTest(scheme=scheme, tier=tier):
                    self.assertRegex(palette[tier], r"^#[0-9a-f]{6}$")
                    self.assertIn(f".tc-glass-{name} {{ background-color: @tc_glass_{generate.snake(tier)};", css)

    def test_tiers_are_the_nine_the_mac_names(self):
        self.assertEqual([t[0] for t in generate.GLASS_TIERS],
                         ["pane", "card", "cardQuiet", "well", "control", "controlSelected",
                          "popover", "menu", "nodeCard"])

    def test_radii_follow_the_flat_theme(self):
        css = generate.render_glass_css(self.tokens, "light")
        self.assertIn(".tc-glass-pane { background-color: @tc_glass_pane; border-radius: 20px;", css)
        self.assertIn(".tc-glass-card { background-color: @tc_glass_card; border-radius: 16px;", css)

    def test_every_type_role_has_a_class_relative_to_the_body(self):
        css = generate.render_glass_css(self.tokens, "dark")
        for role in self.tokens["type"]:
            self.assertIn(".tc-type-" + generate.snake(role).replace("_", "-") + " {", css)
        self.assertNotRegex(css, r"font-size: [\d.]+px")

    def test_unreadable_future_ink_is_rejected(self):
        self.tokens["themes"]["flatDark"]["color"]["textSecondary"] = {"hex": "#0d1124"}
        with self.assertRaisesRegex(generate.TokenError, "inkSecondary"):
            generate.glass_palette(self.tokens, "dark")

    def test_pane_ground_carries_the_pane_fill(self):
        # Every Mac pane path paints paneFill over its base (GlassPaneFill),
        # so the painted ground is paneFill over paneOpaque, and the other
        # tiers are flattened over that ground, not over the bare base.
        for scheme in generate.SCHEMES:
            with self.subTest(scheme=scheme):
                palette = generate.glass_palette(self.tokens, scheme)
                base = generate.flat_color(self.tokens, "paneOpaque", scheme)["hex"]
                ground = generate.opaque_over(generate.flat_fill(self.tokens, "paneFill", scheme), base)
                self.assertEqual(palette["pane"], ground)
                self.assertEqual(palette["card"], generate.opaque_over(
                    generate.flat_fill(self.tokens, "cardFill", scheme), ground))
        self.assertNotEqual(generate.glass_palette(self.tokens, "dark")["pane"], "#0d1124")

    def test_inactive_window_dims_the_panes(self):
        # GlassPaneFill's inactiveDim when the window is not key; GTK marks
        # an unfocused toplevel :backdrop.
        for scheme in generate.SCHEMES:
            with self.subTest(scheme=scheme):
                palette = generate.glass_palette(self.tokens, scheme)
                dim = generate.flat_color(self.tokens, "inactiveDim", scheme)
                self.assertEqual(palette["paneInactive"], generate.opaque_over(dim, palette["pane"]))
                self.assertNotEqual(palette["paneInactive"], palette["pane"])
                css = generate.render_glass_css(self.tokens, scheme)
                self.assertIn("@define-color tc_glass_pane_inactive " + palette["paneInactive"] + ";", css)
                self.assertIn("window.tc-glass-window:backdrop .tc-glass-pane { "
                              "background-color: @tc_glass_pane_inactive; }", css)

    def test_inactive_pane_keeps_text_readable(self):
        self.tokens["themes"]["flatDark"]["color"]["inactiveDim"] = {"hex": "#f2f3f8", "alpha": 0.8}
        with self.assertRaisesRegex(generate.TokenError, "paneInactive"):
            generate.glass_palette(self.tokens, "dark")

    def test_high_contrast_strokes_only_the_tiers_with_edges(self):
        # GlassEdgeModifier's increased branch: a tier with edge layers gets
        # a solid 1pt edgeHighContrast stroke; flat cards have none.
        for scheme in generate.SCHEMES:
            with self.subTest(scheme=scheme):
                palette = generate.glass_palette(self.tokens, scheme)
                css = generate.render_glass_css(self.tokens, scheme)
                edge = generate.flat_color(self.tokens, "edgeHighContrast", scheme)
                self.assertEqual(palette["edgeHighContrast"], generate.css_rgba(edge, "test"))
                stroked = sorted(line.split(" ")[1] for line in css.splitlines()
                                 if line.startswith("window.tc-high-contrast"))
                self.assertEqual(stroked, sorted(f".tc-glass-{name}" for name in
                                                 ("pane", "popover", "menu", "node-card")))

    def test_tab_strip_takes_the_rule_and_the_ember_halo(self):
        # GlassRuleTabs: a 1pt `rule` under the strip and a `tabEmberHalo`
        # glow under the selected tab's title.
        for scheme in generate.SCHEMES:
            with self.subTest(scheme=scheme):
                palette = generate.glass_palette(self.tokens, scheme)
                css = generate.render_glass_css(self.tokens, scheme)
                rule = generate.opaque_over(generate.flat_color(self.tokens, "rule", scheme), palette["pane"])
                self.assertEqual(palette["rule"], rule)
                self.assertIn(f"@define-color tc_glass_rule {rule};", css)
                halo = generate.css_rgba(generate.flat_color(self.tokens, "tabEmberHalo", scheme), "test")
                self.assertIn(f"@define-color tc_glass_tab_ember_halo {halo};", css)

    def test_graph_inks_reach_rust_per_scheme(self):
        # GlassBarGraph's lit dots: dataShared over dataKept, each scheme's
        # flat value, for the cairo graph to draw with.
        rust = "\n".join(generate.render_glass_rust(self.tokens))
        for scheme in generate.SCHEMES:
            for token, name in (("dataShared", "SHARED"), ("dataKept", "KEPT"), ("statusOff", "LABEL")):
                hex_ = generate.flat_color(self.tokens, token, scheme)["hex"].upper()
                self.assertIn(f'pub const {name}_{scheme.upper()}: &str = "{hex_}";', rust)

    def test_unfocused_window_dims_every_tier_as_the_macs_material_path(self):
        # GlassPaneFill's material path fills inactiveDim under the pane's
        # translucent cards, wells and controls, so they dim with the pane.
        # Linux paints every tier opaque over the pane, so each tier's
        # unfocused fill is the tier over the dimmed pane. Expected values
        # are worked by hand from GlassTokens.swift's flat theme:
        # paneOpaque (:129) + paneFill (:410) + inactiveDim (:135), then
        # cardFill (:411), wellFill (:136) over the card, rule (:176).
        expected = {
            "light": {"pane": "#e8e9ec", "card": "#f6f6f7", "well": "#d8dae0", "rule": "#cccdd0"},
            "dark": {"pane": "#15192a", "card": "#121628", "well": "#0d0f1c", "rule": "#313544"},
        }
        for scheme in generate.SCHEMES:
            with self.subTest(scheme=scheme):
                css = generate.render_glass_css(self.tokens, scheme)
                for name, value in expected[scheme].items():
                    self.assertIn(f"@define-color tc_glass_{name}_inactive {value};", css)
                # Every tier, not only the pane, has its unfocused fill.
                for tier, css_name, _, _, _ in generate.GLASS_TIERS:
                    self.assertIn(
                        f"window.tc-glass-window:backdrop .tc-glass-{css_name} "
                        f"{{ background-color: @tc_glass_{generate.snake(tier)}_inactive; }}", css)

    def test_unfocused_window_dims_insights(self):
        # Insights paints its own opaque ground and cards; unfocused, they
        # take the dimmed pane's values (hand-worked above), so Insights is
        # not an undimmed block in a dimmed pane.
        expected = {"light": ("#e8e9ec", "#f6f6f7"), "dark": ("#15192a", "#121628")}
        for scheme in generate.SCHEMES:
            with self.subTest(scheme=scheme):
                ground, card = expected[scheme]
                css = generate.render_insights_css(self.tokens, scheme)
                self.assertIn(f"window.tc-glass-window:backdrop .tc-insights {{ background: {ground}; }}", css)
                self.assertIn("window.tc-glass-window:backdrop .tc-insights .tc-insights-card, "
                              "window.tc-glass-window:backdrop .tc-insights button.tc-insights-summary "
                              f"{{ background: {card}; }}", css)

    def test_high_contrast_edge_is_an_overlay_not_a_border(self):
        # GlassEdgeModifier strokes the tier's shape: an overlay that takes
        # no layout space. A CSS border would grow the widget by 1px a side,
        # so the stroke is an inset shadow drawn with the tier's own shadow.
        for scheme in generate.SCHEMES:
            with self.subTest(scheme=scheme):
                css = generate.render_glass_css(self.tokens, scheme)
                lines = [line for line in css.splitlines() if line.startswith("window.tc-high-contrast")]
                self.assertTrue(lines)
                for line in lines:
                    self.assertNotIn("border", line)
                    self.assertIn("box-shadow: inset 0 0 0 1px @tc_glass_edge_high_contrast, ", line)
                pane = [line for line in lines if line.startswith("window.tc-high-contrast .tc-glass-pane ")]
                self.assertEqual(len(pane), 1)
                # The pane keeps its drop shadow under the stroke.
                drop = {"light": "0px 14px 30px rgba(0, 0, 0, 0.12)", "dark": "0px 18px 36px rgba(2, 3, 10, 0.6)"}
                self.assertTrue(pane[0].endswith(f"{drop[scheme]}; }}"), pane[0])

    def test_status_and_graph_inks_are_the_macs_flat_values(self):
        # GlassTokens.swift: statusOn (:52), statusAsk (:54), statusOff
        # (:56), statusOutside (:58), flat theme; graphFocusOn/Idle/Off
        # (:232-236); textPrimary (:31); GlassColor.ink is pure black in
        # light and white in dark (:252). Light statusOff was #8A8A90, raised
        # to pass on the map (owner, 2026-11-20: "Raise to pass, both apps";
        # LightContrastRaiseTests).
        rust = "\n".join(generate.render_glass_rust(self.tokens))
        for name, light, dark in (
            ("ON", "#1A8F4C", "#30D158"), ("ASK", "#A8690F", "#FFC75C"),
            ("OFF", "#78787F", "#A9A9B0"), ("OUTSIDE", "#D0342C", "#FF6B6B"),
        ):
            self.assertIn(f'pub const {name}_LIGHT: &str = "{light}";', rust)
            self.assertIn(f'pub const {name}_DARK: &str = "{dark}";', rust)
        self.assertIn('pub const TEXT_PRIMARY_LIGHT: &str = "#1D1D1F";', rust)
        # GlassRuleTabs' ember and halo (tabEmber :178, tabEmberHalo :180).
        self.assertIn('pub const EMBER_LIGHT: &str = "#056BDC";', rust)
        self.assertIn('pub const EMBER_DARK: &str = "#0091FD";', rust)
        self.assertIn('pub const HALO_LIGHT: (&str, f64) = ("#056BDC", 0.28);', rust)
        self.assertIn('pub const HALO_DARK: (&str, f64) = ("#0091FD", 0.31);', rust)
        self.assertIn('pub const TEXT_PRIMARY_DARK: &str = "#F2F3F8";', rust)
        for scheme, values in (
            ("light", ("#1f6fd1", "#3c3c42", "#a1a1a6", "rgba(0, 0, 0, 0.07)")),
            ("dark", ("#4da3ff", "#d6d6dc", "#6b6b70", "rgba(255, 255, 255, 0.07)")),
        ):
            with self.subTest(scheme=scheme):
                css = generate.render_glass_css(self.tokens, scheme)
                on, idle, off, range_fill = values
                self.assertIn(f"@define-color tc_glass_graph_focus_on {on};", css)
                self.assertIn(f"@define-color tc_glass_graph_focus_idle {idle};", css)
                self.assertIn(f"@define-color tc_glass_graph_focus_off {off};", css)
                # The range capsule's fill (MonitorShell.swift:255).
                self.assertIn(f"@define-color tc_glass_range_fill {range_fill};", css)

    def test_summary_glyph_inks_are_the_macs_text_tones(self):
        # SummaryDecisionLine: a glyph in its status's text colour
        # (statusOnText :60, statusAskText :62), or textTertiary (:34)
        # with no status; flat theme, flattened over the pane.
        for scheme, (on, ask, tertiary) in (
            ("light", ("#0b4628", "#573403", "#3d3d43")),
            ("dark", ("#acedbc", "#ffd585", "#cfd0d8")),
        ):
            with self.subTest(scheme=scheme):
                css = generate.render_glass_css(self.tokens, scheme)
                self.assertIn(f"@define-color tc_glass_status_on_text {on};", css)
                self.assertIn(f"@define-color tc_glass_status_ask_text {ask};", css)
                self.assertIn(f"@define-color tc_glass_ink_tertiary {tertiary};", css)

    def test_tree_row_inks_are_the_macs(self):
        # GlassListRow (ListRow.swift): a flagged sub-line in statusAsk or
        # statusOn (:292-296), the chevron in statusOff (:149), and
        # GlassToolTile's folder tile on tileFolder (Indicators.swift:306);
        # flat theme, flattened over the pane. Light off raised from #8a8a90
        # (owner, 2026-11-20: "Raise to pass, both apps").
        for scheme, (ask, on, off, folder) in (
            ("light", ("#a8690f", "#1a8f4c", "#78787f", "#e6e6ea")),
            ("dark", ("#ffc75c", "#30d158", "#a9a9b0", "#2a2a2c")),
        ):
            with self.subTest(scheme=scheme):
                css = generate.render_glass_css(self.tokens, scheme)
                self.assertIn(f"@define-color tc_glass_status_ask {ask};", css)
                self.assertIn(f"@define-color tc_glass_status_on {on};", css)
                self.assertIn(f"@define-color tc_glass_status_off {off};", css)
                self.assertIn(f"@define-color tc_glass_tile_folder {folder};", css)

    def test_glyphs_come_from_the_swift_path_data(self):
        glyphs = generate.parse_glyphs(generate.GLYPHS.read_text())
        self.assertEqual([g["name"] for g in glyphs],
                         ["viewMenu", "graph", "map", "inspector", "gear", "chevronDown", "folder", "kebab"])
        by_name = {g["name"]: g for g in glyphs}
        self.assertEqual(by_name["viewMenu"]["data"], "M1,2h12M1,6h12M1,10h12")
        # The shared frame is prepended, as Swift does with Self.frame.
        self.assertTrue(by_name["graph"]["data"].startswith("M2.5,0.5H11.5A2,2,0,0,1,13.5,2.5"))
        self.assertTrue(by_name["graph"]["data"].endswith("M0.5,7.5h13M4,7.5v4M8,7.5v4"))
        self.assertIsNone(by_name["kebab"]["stroke"])
        self.assertEqual(by_name["gear"]["size"], (15, 15))
        self.assertIn('fill="currentColor"', generate.glyph_svg(by_name["kebab"]))
        self.assertIn('stroke="currentColor"', generate.glyph_svg(by_name["map"]))

    def test_logos_come_from_the_swift_path_data(self):
        logos = generate.parse_logos(generate.LOGOS.read_text())
        self.assertEqual([l["name"] for l in logos], ["claude", "codex", "antigravity", "openCode", "redTeam", "theia"])
        by_name = {l["name"]: l for l in logos}
        self.assertEqual(by_name["openCode"]["view_box"], (0, 0, 24, 24))
        self.assertEqual(by_name["openCode"]["paths"], ["M16,6H8v12h8V6zm4,16H4V2h16v20z"])
        self.assertEqual(by_name["theia"]["view_box"], (0, 0, 64, 64))
        self.assertEqual(len(by_name["theia"]["paths"]), 3)
        self.assertTrue(by_name["claude"]["paths"][0].startswith("M4.709,15.955l4.72-2.647"))

    def test_brand_mark_comes_from_the_swift_path_data(self):
        mark = generate.parse_brand_mark(generate.BRAND_MARK.read_text())
        self.assertEqual(mark["view_box"], (82.96, 82.13, 234.08, 231.72))
        self.assertTrue(mark["data"].startswith("M292.106,82.1357C283.445,82.1357"))
        self.assertTrue(mark["data"].endswith("Z"))

    def test_map_colours_follow_the_flat_theme(self):
        rendered = "\n".join(generate.render_map_rust(self.tokens))
        # Flat light overrides the shared orb and purpleSoft with its blue.
        self.assertIn('pub const MAP_ORB_SHARED: Rgba = ("#056bdc", 1.0);', rendered)
        self.assertIn('pub const PURPLE_SOFT: Rgba = ("#0091fd", 1.0);', rendered)
        self.assertIn('pub const WELL_FILL: Rgba = ("#8a93a5", 0.28);', rendered)
        self.assertIn('pub const TOOL_CLAUDE: Rgba = ("#c2603e", 1.0);', rendered)
        self.assertIn('pub const TOOL_CLAUDE: Rgba = ("#e08a6a", 1.0);', rendered)
        self.assertIn('pub const HERO_CAMPAIGN_DEEP: Rgba = ("#0057b0", 1.0);', rendered)
        self.assertIn('pub const HERO_CAMPAIGN_BRIGHT: Rgba = ("#0091fd", 1.0);', rendered)

    def test_generated_output_is_deterministic(self):
        first = generate.render_gtk(self.tokens)
        self.assertEqual(first, generate.render_gtk(self.tokens))


class WindowsGlassTokensTests(unittest.TestCase):
    """The WinUI app's glass look (spec W5)."""

    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def test_windows_roles_are_the_flat_blue_not_the_purple(self):
        roles = {s: dict((r, h) for r, _, h in rows) for s, rows in generate.windows_role_values(self.tokens).items()}
        # GlassTokens.swift:415 ctaFill flat, :178 tabEmber flat.
        self.assertEqual(roles["dark"]["accent"], "#7EBCFF")
        self.assertEqual(roles["light"]["accent"], "#0558B6")
        self.assertEqual(roles["dark"]["accentLine"], "#0091FD")
        xaml = generate.render_windows(self.tokens)
        self.assertNotIn("8A3DFF", xaml)
        self.assertNotIn("6D14F3", xaml)

    def test_gtk_keeps_its_own_roles(self):
        # The Windows change leaves the shared SHELL_ROLES, and so GTK, alone.
        self.assertEqual(generate.SHELL_ROLES[0], ("accent", "purple", "purpleSoft"))

    def test_every_tier_and_its_inactive_twin_reach_the_xaml(self):
        xaml = generate.render_windows_glass(self.tokens)
        for tier, *_ in generate.GLASS_TIERS:
            key = "TcGlass" + generate.pascal(tier)
            self.assertIn(f'x:Key="{key}Brush"', xaml)
            self.assertIn(f'x:Key="{key}InactiveBrush"', xaml)

    def test_motion_reaches_xaml_and_cs(self):
        xaml = generate.render_windows_glass(self.tokens)
        cs = generate.render_windows_glass_cs(self.tokens)
        self.assertIn('<x:Double x:Key="TcGlassMotionFast">0.15</x:Double>', xaml)
        self.assertIn("public const double Spin = 0.8;", cs)

    def test_unreadable_status_text_is_refused(self):
        self.tokens["themes"]["flatDark"]["color"]["statusAskText"] = {"hex": "#3a3a3a"}
        with self.assertRaisesRegex(generate.TokenError, "statusAskText"):
            generate.windows_glass_palette(self.tokens, "dark")

    def test_high_contrast_still_marks_the_tab_shown(self):
        # The Mac draws the ember under the tab shown in every appearance
        # (Navigation.swift:288). Under High Contrast the box showed no mark
        # at all: the ember's colours were Transparent. The line takes the
        # system's highlight there, solid end to end; the halo stays off.
        xaml = generate.render_windows_glass(self.tokens)
        hc = xaml[xaml.index('<ResourceDictionary x:Key="HighContrast">'):]
        hc = hc[:hc.index("</ResourceDictionary>")]
        for key in ("TcGlassTabEmberColor", "TcGlassTabEmberClearColor"):
            self.assertIn(f'<StaticResource x:Key="{key}" ResourceKey="SystemColorHighlightColor" />', hc)
        for key in ("TcGlassTabEmberHaloColor", "TcGlassTabEmberHaloClearColor"):
            self.assertIn(f'<Color x:Key="{key}">Transparent</Color>', hc)

    def test_high_contrast_keeps_every_label_off_its_fill(self):
        # Each pair the light and dark checks hold to 4.5:1 must also differ
        # under High Contrast: a label on the system's ground over a fill on
        # the system's ground is the same colour on itself. The selected
        # segment (Navigation.swift:121-123, textOnSelected on
        # controlSelected) was ground on ground until W25.
        xaml = generate.render_windows_glass(self.tokens)
        hc = xaml[xaml.index('<ResourceDictionary x:Key="HighContrast">'):]
        hc = hc[:hc.index("</ResourceDictionary>")]

        def system(key):
            marker = f'<StaticResource x:Key="{key}" ResourceKey="'
            start = hc.index(marker) + len(marker)
            return hc[start:hc.index('"', start)]

        for fill, label in generate.WINDOWS_GLASS_LABEL_PAIRS:
            for twin in (fill, fill.replace("Brush", "InactiveBrush")):
                if f'x:Key="{twin}"' not in hc:
                    continue
                self.assertNotEqual(system(label), system(twin), f"{label} on {twin} under High Contrast")

    def test_veil_over_material_is_translucent(self):
        palette = generate.windows_glass_palette(self.tokens, "dark")
        self.assertNotEqual(palette["TcGlassPaneOverMaterialBrush"][1:3], "FF")


def rgba(text):
    """`rgba(r, g, b, a)` as four floats."""
    inner = text.strip()
    assert inner.startswith("rgba(") and inner.endswith(")"), text
    r, g, b, a = (float(v) for v in inner[5:-1].split(","))
    return r, g, b, a


def stack(*layers):
    """Source-over compositing of (hex, alpha) layers, bottom first, onto
    nothing: the colour and alpha one layer must carry to look the same."""
    r = g = b = a = 0.0
    for hex_, alpha in layers:
        c = [int(hex_[i:i + 2], 16) for i in (1, 3, 5)]
        out = alpha + a * (1 - alpha)
        r, g, b = ((c[i] * alpha + (r, g, b)[i] * a * (1 - alpha)) / out for i in range(3))
        a = out
    return r, g, b, a


def over_ground(colour, ground):
    """An rgba tuple flattened over an opaque hex ground, as hex."""
    r, g, b, a = colour
    base = [int(ground[i:i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join(f"{round(c * a + base[i] * (1 - a)):02x}" for i, c in enumerate((r, g, b)))


class GlassBlurTokensTests(unittest.TestCase):
    """The translucent window KDE blur shows through. Values pinned to the
    Mac's flat theme: macos/Sources/TCDesign/Generated/GlassTokens.swift and
    Tokens/GlassTheme.swift."""

    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def assertRgba(self, text, expected):
        got = rgba(text)
        for g, e in zip(got[:3], expected[:3]):
            self.assertAlmostEqual(g, e, delta=0.51, msg=text)
        self.assertAlmostEqual(got[3], expected[3], delta=0.0006, msg=text)

    def test_tint_share_is_the_macs_in_dark_and_the_smallest_readable_lift_in_light(self):
        # GlassTheme.swift:82 `glassTintShare = 0.25`: the Liquid Glass tint
        # is the veil at a quarter of its alpha. Dark reads at that share
        # over any desktop; light's secondary text over a black desktop,
        # unfocused, does not (4.38:1). 0.30 is the first step that does
        # (4.54:1).
        self.assertEqual(generate.GLASS_TINT_SHARE, 0.25)
        self.assertEqual(generate.glass_blur_tint_share(self.tokens, "dark"), 0.25)
        self.assertEqual(generate.glass_blur_tint_share(self.tokens, "light"), 0.3)

    def test_pane_is_the_macs_material_stack_in_one_colour(self):
        # GlassPaneFill (GlassBackdrop.swift:156-170): the glass, tinted with
        # the veil at the tint share, then glassVeil, then paneFill, and
        # inactiveDim over all of it out of focus. GlassTokens.swift:131
        # glassVeil flat: 0x070A1A @0.68, light 0xE3E6EC @0.66.
        cases = {
            "dark": (("#070a1a", 0.68), 0.25, ("#ffffff", 0.04), ("#070a1a", 0.14)),
            "light": (("#e3e6ec", 0.66), 0.3, ("#f4f6f9", 0.08), ("#000000", 0.04)),
        }
        for scheme, (veil, share, pane_fill, dim) in cases.items():
            with self.subTest(scheme=scheme):
                palette = generate.glass_blur_palette(self.tokens, scheme)
                tint = (veil[0], veil[1] * share)
                self.assertRgba(palette["pane"], stack(tint, veil, pane_fill))
                self.assertRgba(palette["paneInactive"], stack(tint, veil, pane_fill, dim))

    def test_tiers_inside_a_pane_take_the_flat_fills_with_their_alpha(self):
        # GlassTier.fill (GlassStyle.swift:207-233): everything inside a pane
        # is painted with the token's own alpha over the pane, and dims with
        # the pane beneath it rather than with a fill of its own.
        cases = {
            "dark": {"card": ("#0d1124", 0.35), "cardQuiet": ("#0d1124", 0.28),
                     "well": ("#000000", 0.3), "control": ("#000000", 0.3),
                     "rule": ("#ffffff", 0.12)},
            "light": {"card": ("#ffffff", 0.6), "cardQuiet": ("#ffffff", 0.5),
                      "well": ("#8a93a5", 0.28), "control": ("#ffffff", 0.7),
                      "rule": ("#000000", 0.12)},
        }
        for scheme, tiers in cases.items():
            palette = generate.glass_blur_palette(self.tokens, scheme)
            for tier, layer in tiers.items():
                with self.subTest(scheme=scheme, tier=tier):
                    self.assertRgba(palette[tier], stack(layer))
                    self.assertEqual(palette[tier + "Inactive"], palette[tier])

    def test_separate_surfaces_and_unblurred_floats_stay_painted(self):
        # A popover and a menu are their own surfaces, with no blur behind
        # them; a node card floats over the map, which the Mac blurs within
        # the window (GlassFloatingBlur) and Linux does not. All three keep
        # their opaque painted fills.
        for scheme in generate.SCHEMES:
            css = generate.render_glass_blur_css(self.tokens, scheme)
            for name in ("popover", "menu", "node_card"):
                with self.subTest(scheme=scheme, tier=name):
                    self.assertNotIn(f"@define-color tc_glass_{name} ", css)
                    self.assertNotIn(f"@define-color tc_glass_{name}_inactive ", css)

    def test_the_window_is_clear_and_draws_no_frame_across_the_gaps(self):
        # GlassBackdrop.swift:219-227: not opaque, no window shadow; the
        # desktop shows in the gaps. libadwaita's window.csd outline and
        # 1px ring would draw a rectangle across them; the shadow's extent
        # stays, transparent, as the resize margin.
        for scheme in generate.SCHEMES:
            css = generate.render_glass_blur_css(self.tokens, scheme)
            with self.subTest(scheme=scheme):
                self.assertIn("@define-color tc_glass_scene transparent;", css)
                self.assertIn("window.tc-glass-window.csd, window.tc-glass-window.csd:backdrop "
                              "{ outline: none; box-shadow: 0 0 14px 5px transparent; }", css)
                self.assertIn("window.tc-glass-window.maximized, window.tc-glass-window.fullscreen "
                              "{ box-shadow: none; }", css)

    def test_text_reads_over_a_black_or_white_desktop_in_both_focus_states(self):
        for scheme in generate.SCHEMES:
            palette = generate.glass_palette(self.tokens, scheme)
            blur = generate.glass_blur_palette(self.tokens, scheme)
            for desktop in ("#000000", "#ffffff"):
                for focus in ("pane", "paneInactive"):
                    ground = over_ground(rgba(blur[focus]), desktop)
                    surfaces = {"pane": ground}
                    for tier in ("card", "cardQuiet", "control"):
                        surfaces[tier] = over_ground(rgba(blur[tier]), ground)
                    for surface, colour in surfaces.items():
                        for ink in ("ink", "inkSecondary"):
                            with self.subTest(scheme=scheme, desktop=desktop, focus=focus,
                                              surface=surface, ink=ink):
                                self.assertGreaterEqual(
                                    generate.contrast(palette[ink], colour), 4.5)

    def test_a_veil_too_thin_to_read_over_the_desktop_is_rejected(self):
        self.tokens["themes"]["flatDark"]["color"]["glassVeil"] = {"hex": "#070a1a", "alpha": 0.2}
        with self.assertRaisesRegex(generate.TokenError, "translucent"):
            generate.glass_blur_palette(self.tokens, "dark")

    def test_insights_lets_the_pane_show_and_fills_with_alpha(self):
        # Insights sits in a pane: its ground is the pane's, so it paints
        # none of its own over the translucent pane, and its cards and
        # switcher take cardFill and controlFill with their alpha.
        for scheme in generate.SCHEMES:
            css = generate.render_insights_blur_css(self.tokens, scheme)
            card = generate.css_rgba(generate.flat_fill(self.tokens, "cardFill", scheme), "cardFill")
            control = generate.css_rgba(generate.flat_fill(self.tokens, "controlFill", scheme), "controlFill")
            glass = "window.tc-glass-window"
            with self.subTest(scheme=scheme):
                self.assertIn(f"{glass} .tc-insights {{ background: transparent; }}", css)
                self.assertIn(f"{glass} .tc-insights .tc-insights-card, "
                              f"{glass} .tc-insights button.tc-insights-summary "
                              f"{{ background: {card}; }}", css)
                self.assertIn(f"{glass} .tc-insights stackswitcher {{ background: {control}; }}", css)
                # The unfocused twins the painted sheet carries are undone.
                self.assertIn(f"{glass}:backdrop .tc-insights {{ background: transparent; }}", css)
                # The sheet is display-wide while loaded: an Insights window
                # of its own keeps its ground, so every rule is scoped.
                for line in css.splitlines():
                    for selector in line.split("{")[0].split(","):
                        self.assertTrue(selector.strip().startswith(glass), selector)

    def test_blur_sheets_reach_rust(self):
        out = generate.render_gtk(self.tokens)
        for name in ("GLASS_LIGHT_BLUR_CSS", "GLASS_DARK_BLUR_CSS",
                     "INSIGHTS_LIGHT_BLUR_CSS", "INSIGHTS_DARK_BLUR_CSS"):
            self.assertIn(f"pub const {name}: &str", out)


class HeroTokensTests(unittest.TestCase):
    """Home's hero cards on Linux take the Mac's flat values from the JSON."""

    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def test_grounds_are_the_card_tokens_not_the_map_gradient(self):
        hero = "\n".join(generate.render_hero_rust(self.tokens))
        self.assertIn('HERO_CAMPAIGN_CARD: Ground = (120, &[(0.0, "#081538"), (1.0, "#081538")])', hero)
        self.assertIn('HERO_MISSIONS: Ground = (120, &[(0.0, "#07152f"), (1.0, "#07152f")])', hero)
        self.assertIn('HERO_INSIGHTS: Ground = (120, &[(0.0, "#0a2546"), (1.0, "#0a2546")])', hero)
        self.assertIn('TINTS_DARK: [&str; 3] = ["#3a7bd5", "#0091fd", "#e2f6ff"]', hero)
        self.assertIn('TINTS_LIGHT: [&str; 3] = ["#2f6bc4", "#2a78d6", "#9fdcff"]', hero)

    def test_a_translucent_or_themed_ground_is_refused(self):
        self.tokens['gradient']['heroMissions']['stops'][0]['alpha'] = 0.5
        with self.assertRaisesRegex(generate.TokenError, 'heroMissions'):
            generate.render_hero_rust(self.tokens)

    def test_a_translucent_ink_is_refused(self):
        self.tokens['color']['heroInk']['alpha'] = 0.9
        with self.assertRaisesRegex(generate.TokenError, 'heroInk'):
            generate.render_hero_rust(self.tokens)


    def test_tile_values_are_the_macs(self):
        rendered = "\n".join(generate.render_tile_rust(self.tokens))
        # GlassTokens.Size.toolTile / toolTileLarge / toolLogo / toolLogoLarge.
        for line in ("TOOL_TILE: f64 = 22.0", "TOOL_TILE_LARGE: f64 = 30.0",
                     "TOOL_LOGO: f64 = 15.0", "TOOL_LOGO_LARGE: f64 = 20.0",
                     "SPINNER: f64 = 12.0", "SPINNER_STROKE: f64 = 1.5", "SPIN: f64 = 0.8",
                     "PULSE: f64 = 1.6", "SKELETON_DIM: f64 = 0.5"):
            self.assertIn(f"pub const {line};", rendered)
        # The flat theme rounds a control at 11, the classic at 8.
        self.assertIn("pub const RADIUS_CONTROL: f64 = 11.0;", rendered)
        self.assertIn("pub const RADIUS_TILE: f64 = 6.0;", rendered)
        self.assertIn('pub const TILE_FOLDER: Rgba = ("#e6e6ea", 1.0);', rendered)
        self.assertIn('pub const TILE_FOLDER: Rgba = ("#2a2a2c", 1.0);', rendered)
        self.assertIn('pub const TILE_FOLDER_INK: Rgba = ("#8be28b", 1.0);', rendered)



class SwitchTokensTests(unittest.TestCase):
    """GlassToggleStyle in the flat theme, for every GTK switch."""

    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def test_on_track_and_knobs_are_the_macs_flat_values(self):
        for scheme, track, knob_on, knob_off in [("light", "#0558B6", "#FFFFFF", "#FFFFFF"),
                                                 ("dark", "#7EBCFF", "#071A2E", "#071A2E")]:
            css = generate.render_glass_css(self.tokens, scheme)
            with self.subTest(scheme=scheme):
                self.assertIn(f"@define-color tc_glass_toggle_on {track};".lower(), css.lower())
                palette = generate.glass_palette(self.tokens, scheme)
                self.assertEqual(palette["toggleOn"].upper(), track)
                self.assertEqual(palette["toggleKnobOn"].upper(), knob_on)
                self.assertEqual(palette["toggleKnobOff"].upper(), knob_off)

    def test_off_track_is_the_macs_in_dark_and_the_smallest_lift_that_holds_3_in_light(self):
        # Dark's 38% white holds 3:1 both ways on every ground. Light's 24%
        # black is about 1.8:1 on its grounds; 43% is the first step at
        # which the track clears 3:1 on every ground and the white knob
        # 3:1 on the track.
        self.assertEqual(generate.switch_off_track(self.tokens, "dark"),
                         {"hex": "#ffffff", "alpha": 0.38})
        self.assertEqual(generate.switch_off_track(self.tokens, "light"),
                         {"hex": "#000000", "alpha": 0.43})
        self.assertIn("@define-color tc_glass_toggle_off rgba(0, 0, 0, 0.43);",
                      generate.render_glass_css(self.tokens, "light"))
        self.assertIn("@define-color tc_glass_toggle_off rgba(255, 255, 255, 0.38);",
                      generate.render_glass_css(self.tokens, "dark"))

    def test_every_switch_state_clears_3_on_every_ground(self):
        for scheme in generate.SCHEMES:
            palette = generate.glass_palette(self.tokens, scheme)
            off = generate.switch_off_track(self.tokens, scheme)
            for ground in generate.SWITCH_GROUNDS:
                under = palette[ground]
                track = generate.opaque_over(off, under)
                with self.subTest(scheme=scheme, ground=ground):
                    self.assertGreaterEqual(generate.contrast(track, under), 3)
                    self.assertGreaterEqual(generate.contrast(palette["toggleKnobOff"], track), 3)
                    self.assertGreaterEqual(generate.contrast(palette["toggleOn"], under), 3)
                    self.assertGreaterEqual(
                        generate.contrast(palette["toggleKnobOn"], palette["toggleOn"]), 3)

    def test_an_on_track_under_the_floor_is_rejected(self):
        self.tokens["themes"]["flatLight"]["color"]["toggleOnSettings"] = {"hex": "#d0d4ff"}
        with self.assertRaisesRegex(generate.TokenError, "switch"):
            generate.render_glass_css(self.tokens, "light")

    def test_the_secondary_ink_reaches_rust(self):
        rendered = "\n".join(generate.render_glass_rust(self.tokens))
        light = generate.glass_palette(self.tokens, "light")["inkSecondary"].upper()
        dark = generate.glass_palette(self.tokens, "dark")["inkSecondary"].upper()
        self.assertEqual(light, "#3C3C43")
        self.assertIn(f'pub const SECONDARY_LIGHT: &str = "{light}";', rendered)
        self.assertIn(f'pub const SECONDARY_DARK: &str = "{dark}";', rendered)

class FirstRunStageTokensTests(unittest.TestCase):
    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def test_first_run_colours_follow_the_flat_theme(self):
        rendered = "\n".join(generate.render_first_run_rust(self.tokens))
        # The flat dark scene is the navy the Mac's first run draws on.
        self.assertIn('pub const SCENE_BASE: Rgba = ("#070a1a", 1.0);', rendered)
        self.assertIn('pub const SCENE_WARM: Rgba = ("#11152b", 1.0);', rendered)
        self.assertIn('pub const FIRST_RUN_GLOW: Rgba = ("#0091fd", 1.0);', rendered)
        self.assertIn('pub const FIRST_RUN_GLOW: Rgba = ("#0070c6", 1.0);', rendered)
        self.assertIn('pub const FIRST_RUN_TILE_SHADOW: Rgba = ("#02030a", 0.5);', rendered)
        self.assertIn('pub const FIRST_RUN_RULE: Rgba = ("#ffffff", 0.18);', rendered)
        self.assertIn('pub const HERO_INK: Rgba = ("#ffffff", 1.0);', rendered)
        self.assertIn('pub const TOOL_CLAUDE: Rgba = ("#e08a6a", 1.0);', rendered)
        # The marker check's well and stroke (`FirstRunMarkerCheck`).
        self.assertIn('pub const TINT_ACCENT: Rgba = ("#0091fd", 0.22);', rendered)
        self.assertIn('pub const TOGGLE_ON: Rgba = ("#7ebcff", 1.0);', rendered)
        self.assertIn('pub const TOGGLE_ON: Rgba = ("#0558b6", 1.0);', rendered)

    def test_first_run_carries_the_motion_curve(self):
        rendered = "\n".join(generate.render_first_run_rust(self.tokens))
        self.assertIn("pub const EASE: [f64; 4] = [0.2, 0.8, 0.2, 1.0];", rendered)


class MotionTokensTests(unittest.TestCase):
    """`brand_tokens::motion`: every `GlassTokens.Motion` value and the two
    springs `GlassMotion.swift` hard-codes, so the GTK shell's motion reads
    the Mac's numbers rather than copies of them."""

    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())

    def test_motion_module_is_the_macs(self):
        rendered = "\n".join(generate.render_motion_rust(
            self.tokens, generate.parse_springs(generate.GLASS_MOTION.read_text())))
        for line in ("FAST: f64 = 0.15", "STANDARD: f64 = 0.22", "SLOW: f64 = 0.3",
                     "REVEAL: f64 = 0.18", "SLIDE: f64 = 0.25", "SPIN: f64 = 0.8",
                     "PULSE: f64 = 1.6",
                     "EASE: [f64; 4] = [0.2, 0.8, 0.2, 1.0]",
                     "SPATIAL: (f64, f64) = (0.9, 0.04)",
                     "EXPRESSIVE: (f64, f64) = (0.78, 0.32)"):
            self.assertIn(f"pub const {line};", rendered)
        self.assertIn('pub const EASE_CSS: &str = "cubic-bezier(0.2, 0.8, 0.2, 1)";', rendered)
        self.assertIn("pub mod motion {", rendered)

    def test_springs_are_read_from_the_swift(self):
        springs = generate.parse_springs(generate.GLASS_MOTION.read_text())
        self.assertEqual(springs, {"spatial": (0.9, 0.04), "expressive": (0.78, 0.32)})

    def test_a_spring_missing_from_the_swift_is_refused(self):
        source = generate.GLASS_MOTION.read_text().replace("func expressive", "func renamed")
        with self.assertRaisesRegex(generate.TokenError, "expressive"):
            generate.parse_springs(source)

    def test_the_modal_scrim_it_fades_is_the_flat_themes(self):
        rendered = "\n".join(generate.render_motion_rust(
            self.tokens, generate.parse_springs(generate.GLASS_MOTION.read_text())))
        self.assertIn("pub const MODAL_SCRIM_BLUR: f64 = 8.0;", rendered)
        self.assertIn("pub const MODAL_SCRIM_LIGHT: f64 = 0.25;", rendered)
        self.assertIn("pub const MODAL_SCRIM_DARK: f64 = 0.4;", rendered)

    def test_the_gtk_output_carries_the_motion_module(self):
        self.assertIn("pub mod motion {", generate.render_gtk(self.tokens))


class LightContrastRaiseTests(unittest.TestCase):
    """The four light-mode values a Windows UI Automation audit measured
    under their floors (owner, 2026-11-20: "Raise to pass, both apps"),
    read back from the generated Mac and Windows outputs."""

    # The Traces map's ground as the audit measured it on the Windows test
    # box, light theme: a little darker than the pane token (#f2f3f6), so
    # the floors below hold on what was drawn, not only on the token.
    MAP_GROUND = "#f0f1f4"

    def setUp(self):
        self.tokens = json.loads(generate.SOURCE.read_text())
        self.rendered = {path: text for path, text in generate.outputs(self.tokens)}

    def swift_flat_light(self, name):
        """(hex, alpha) of a colour token's flat-theme light value in the
        generated GlassTokens.swift."""
        import re
        swift = self.rendered[generate.OUTPUT]
        line = next(l for l in swift.splitlines()
                    if f"static var {name}: GlassRGBA" in l or f"static let {name}: GlassRGBA" in l)
        # A token the themes share has no flat: arm; its one value is the flat's.
        flat = line.split("flat:", 1)[1] if "flat:" in line else line.split("=", 1)[1]
        m = re.search(r"light: GlassRGBA\(0x([0-9A-F]{6}), alpha: ([0-9.]+)\)", flat)
        if m is None:  # no light twin: the flat value serves both
            m = re.search(r"GlassRGBA\(0x([0-9A-F]{6}), alpha: ([0-9.]+)", flat)
        return "#" + m.group(1).lower(), float(m.group(2))

    def map_art_light(self, name):
        """(hex, alpha) of an ink in the generated MapArt.g.cs light table."""
        import re
        cs = self.rendered[generate.WINDOWS_MAP_OUTPUT]
        light = cs.split("Light =", 1)[1].split("Dark =", 1)[0]
        m = re.search(r'\["%s"\] = new MapRgba\((\d+), (\d+), (\d+), ([0-9.]+)\)' % name, light)
        return "#%02x%02x%02x" % tuple(int(m.group(i)) for i in (1, 2, 3)), float(m.group(4))

    def test_switch_off_track_clears_3_to_1_on_mac_and_windows(self):
        # 1. The settings switch off: white knob on #B8B9BB, 1.96:1. The
        # Mac's flat light toggleOff takes the alpha Linux already lifts it
        # to, so all three apps draw one track.
        hex_, alpha = self.swift_flat_light("toggleOff")
        palette = generate.glass_palette(self.tokens, "light")
        self.assertEqual(generate.switch_failures(palette, {"hex": hex_, "alpha": alpha}), [])
        self.assertEqual({"hex": hex_, "alpha": alpha},
                         generate.switch_off_track(self.tokens, "light"))
        win = generate.windows_glass_palette(self.tokens, "light")
        knob, track = "#" + win["TcGlassToggleKnobOffBrush"][3:], "#" + win["TcGlassToggleOffBrush"][3:]
        self.assertGreaterEqual(generate.contrast(knob, track), 3.0)
        self.assertGreaterEqual(generate.contrast(track, palette["pane"]), 3.0)

    def test_selected_row_kebab_ink_clears_3_to_1(self):
        # 2. "Dismiss trace" on a selected row: the Mac draws the kebab in
        # GlassGround.selection.controlInk, textOnAccent, on selection.
        for ink, ground in ((self.swift_flat_light("textOnAccent"), self.swift_flat_light("selection")),
                            (self.map_art_light("textOnAccent"), self.map_art_light("selection"))):
            self.assertEqual(ground[1], 1.0)
            self.assertGreaterEqual(generate.contrast(ink[0], ground[0]), 3.0)

    def test_this_computer_glyph_clears_3_to_1(self):
        # 3. "This computer" on the map: statusOff (FlowMapView.swift
        # drawLaptop/drawPad, TracesMapView.DrawLaptop/DrawPad). Held at
        # 0.85 coverage: the standard map's laptop lid, and the audit's own
        # pixel (#8f8f95, about 0.95). The launch pad, the default style,
        # draws its slab 0.75 to 1 and its tower at 0.7; those stay under.
        for hex_, alpha in (self.swift_flat_light("statusOff"), self.map_art_light("statusOff")):
            for coverage in (1.0, 0.85):
                with self.subTest(hex=hex_, coverage=coverage):
                    drawn = generate.opaque_over({"hex": hex_, "alpha": alpha * coverage}, self.MAP_GROUND)
                    self.assertGreaterEqual(generate.contrast(drawn, self.MAP_GROUND), 3.0)

    def test_ask_folder_ring_clears_3_to_1(self):
        # 4. An Ask folder on the map: its ring in statusAsk (FlowMapView
        # rule(.ask), MapGeometry.Rule). The audit read the node as text;
        # its names are textPrimary, and the ring is a graphic, held at 3:1,
        # which #a8690f already clears (lead, on the owner's 2026-11-20
        # decision: statusAsk stays).
        for hex_, alpha in (self.swift_flat_light("statusAsk"), self.map_art_light("statusAsk")):
            with self.subTest(hex=hex_):
                self.assertEqual(alpha, 1.0)
                self.assertGreaterEqual(generate.contrast(hex_, self.MAP_GROUND), 3.0)

    # The launch pad, the map's default "This computer" (FlowMapView
    # drawPad, TracesMapView.DrawPad, GTK draw_pad): the slab's lightest
    # stop is drawn at 0.75 and the tower's bars at 0.7. Each was statusOff
    # (#78787f: 2.60 and 2.42) and takes its own token, darkened the least
    # that clears 3:1 (lead, on the owner's 2026-11-20 decision).
    PAD = (("mapPadSlab", 0.75), ("mapPadTower", 0.7))

    def test_launch_pad_pieces_clear_3_to_1(self):
        for name, coverage in self.PAD:
            for hex_, alpha in (self.swift_flat_light(name), self.map_art_light(name)):
                with self.subTest(name=name, coverage=coverage, hex=hex_):
                    drawn = generate.opaque_over({"hex": hex_, "alpha": alpha * coverage}, self.MAP_GROUND)
                    self.assertGreaterEqual(generate.contrast(drawn, self.MAP_GROUND), 3.0)

if __name__ == '__main__':
    unittest.main()
