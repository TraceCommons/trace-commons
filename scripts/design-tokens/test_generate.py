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
        # light and white in dark (:252).
        rust = "\n".join(generate.render_glass_rust(self.tokens))
        for name, light, dark in (
            ("ON", "#1A8F4C", "#30D158"), ("ASK", "#A8690F", "#FFC75C"),
            ("OFF", "#8A8A90", "#A9A9B0"), ("OUTSIDE", "#D0342C", "#FF6B6B"),
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

    def test_generated_output_is_deterministic(self):
        first = generate.render_gtk(self.tokens)
        self.assertEqual(first, generate.render_gtk(self.tokens))


if __name__ == '__main__':
    unittest.main()
