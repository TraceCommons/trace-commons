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

    def test_generated_output_is_deterministic(self):
        first = generate.render_gtk(self.tokens)
        self.assertEqual(first, generate.render_gtk(self.tokens))


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


if __name__ == '__main__':
    unittest.main()
