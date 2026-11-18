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
            self.assertTrue(all(line.startswith('.tc-insights') for line in css.splitlines()))
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
        self.assertEqual([l["name"] for l in logos], ["claude", "codex", "antigravity", "openCode", "theia"])
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


if __name__ == '__main__':
    unittest.main()
