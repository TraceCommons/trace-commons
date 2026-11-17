"""The GLib advisory exception must never apply to an unreviewed source."""

import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from dependency_policy import GLIB_ADVISORY, verified_glib_exception


class GlibBackportPolicyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        original = Path(__file__).resolve().parents[3]
        tauri = self.root / "tauri-desktop"
        tauri.mkdir()
        for filename in ("Cargo.toml", "Cargo.lock"):
            shutil.copyfile(original / "tauri-desktop" / filename, tauri / filename)
        shutil.copytree(original / "tauri-desktop/vendor/glib", tauri / "vendor/glib")

    def test_reviewed_backport_is_accepted(self):
        self.assertEqual(verified_glib_exception(self.root), GLIB_ADVISORY)

    def test_generated_tauri_policy_keeps_reviewed_maintenance_exceptions(self):
        original = Path(__file__).resolve().parents[3]
        output = self.root / "deny-tauri.toml"
        result = subprocess.run(
            [sys.executable, str(original / "scripts/ci/dependency_policy.py"),
             "--output", str(output)],
            text=True, capture_output=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        policy = tomllib.loads(output.read_text())
        ignored = {entry["id"] for entry in policy["advisories"]["ignore"]}
        self.assertTrue({"RUSTSEC-2026-0543", "RUSTSEC-2026-0555",
                         "RUSTSEC-2024-0429"}.issubset(ignored))

    def test_reverting_pointer_fix_is_rejected(self):
        source = self.root / "tauri-desktop/vendor/glib/src/variant_iter.rs"
        text = source.read_text()
        self.assertIn("&mut p", text)
        source.write_text(text.replace("&mut p", "&p"))
        with self.assertRaisesRegex(ValueError, "reviewed security backport"):
            verified_glib_exception(self.root)

    def test_removing_patch_is_rejected(self):
        manifest = self.root / "tauri-desktop/Cargo.toml"
        manifest.write_text(manifest.read_text().replace('path = "vendor/glib"', 'path = "elsewhere"'))
        with self.assertRaisesRegex(ValueError, "reviewed vendored GLib patch"):
            verified_glib_exception(self.root)

    def test_registry_glib_in_lockfile_is_rejected(self):
        lock = self.root / "tauri-desktop/Cargo.lock"
        original = lock.read_text()
        lock.write_text(original.replace('name = "glib"', 'name = "glib"\nsource = "registry+https://github.com/rust-lang/crates.io-index"'))
        with self.assertRaisesRegex(ValueError, "patched local GLib"):
            verified_glib_exception(self.root)


if __name__ == "__main__":
    unittest.main()
