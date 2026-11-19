"""Tests for gallery.py: run with
python3 -m unittest discover -s scripts/parity -p 'test_*.py'"""

import base64
import os
import tempfile
import unittest

import gallery

# A 1x1 PNG, enough for the page to embed.
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
)


def touch(directory, name):
    with open(os.path.join(directory, name), "wb") as f:
        f.write(PNG)


class ParseTests(unittest.TestCase):
    def test_a_capture_name_parses_into_its_four_parts(self):
        self.assertEqual(
            gallery.parse("normalDay-traces-main+map+inspector-dark.png"),
            gallery.Key("normalDay", "traces", "main+map+inspector", "dark"),
        )
        self.assertEqual(
            gallery.parse("empty-home-main-light.png"),
            gallery.Key("empty", "home", "main", "light"),
        )

    def test_names_outside_the_grammar_do_not_parse(self):
        for name in [
            "normalDay-traces-main-light.jpg",
            "normalDay-settings-main-light.png",
            "normalDay-traces-map-light.png",
            "normalDay-traces-main+inspector+map-light.png",
            "normalDay-traces-main-dim.png",
            "normal-day-traces-main-light.png",
            "macos-shell-traces-tree.png",
        ]:
            self.assertIsNone(gallery.parse(name), name)


class PairTests(unittest.TestCase):
    def test_captures_pair_by_key_and_the_rest_are_listed(self):
        with tempfile.TemporaryDirectory() as mac, tempfile.TemporaryDirectory() as linux:
            touch(mac, "normalDay-traces-main+map-light.png")
            touch(linux, "normalDay-traces-main+map-light.png")
            touch(mac, "empty-home-main-dark.png")
            touch(linux, "busyQueue-insights-main-light.png")
            touch(linux, "screenshot-1.png")
            result = gallery.collect(mac, linux)
        self.assertEqual(
            [p.key for p in result.pairs],
            [gallery.Key("normalDay", "traces", "main+map", "light")],
        )
        self.assertEqual(
            result.mac_only, [gallery.Key("empty", "home", "main", "dark")]
        )
        self.assertEqual(
            result.linux_only, [gallery.Key("busyQueue", "insights", "main", "light")]
        )
        self.assertEqual(result.unparsed, [("linux", "screenshot-1.png")])

    def test_the_page_is_self_contained_and_names_every_unpaired_capture(self):
        with tempfile.TemporaryDirectory() as mac, tempfile.TemporaryDirectory() as linux:
            touch(mac, "normalDay-traces-main+map-light.png")
            touch(linux, "normalDay-traces-main+map-light.png")
            touch(mac, "empty-home-main-dark.png")
            touch(linux, "stray.png")
            page = gallery.render(gallery.collect(mac, linux))
        self.assertIn("data:image/png;base64,", page)
        self.assertNotIn('src="/', page)
        self.assertNotIn("http://", page)
        self.assertNotIn("https://", page)
        self.assertIn("empty-home-main-dark.png", page)
        self.assertIn("stray.png", page)
        self.assertEqual(page.count("<img"), 3)


if __name__ == "__main__":
    unittest.main()
