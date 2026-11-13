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


if __name__ == '__main__':
    unittest.main()
