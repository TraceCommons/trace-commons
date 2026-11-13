#!/usr/bin/python3
"""Reproduce Rust 1.96's malformed release dylib and verify staged workaround."""
import ctypes
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('pilot_archive', ROOT / 'macos/ci_scripts/check-pilot-archive.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class LinkeditRegression(unittest.TestCase):
    def test_pinned_rust_release_layout_before_and_after_staging(self):
        with tempfile.TemporaryDirectory(prefix='tc-linkedit-regression-') as temporary:
            root = Path(temporary)
            (root / 'src').mkdir()
            (root / 'Cargo.toml').write_text('[package]\nname="tc_linkedit_repro"\nversion="0.1.0"\nedition="2024"\n[lib]\ncrate-type=["cdylib"]\n')
            # One indirect symbol makes the affected writer omit four bytes
            # of string-pool padding. This is Rust #157750's minimal case.
            (root / 'src/lib.rs').write_text('unsafe extern "C" { static mut environ: *mut *mut u8; }\n#[unsafe(no_mangle)]\npub extern "C" fn answer() -> usize { unsafe { environ as usize } }\n')
            env = dict(os.environ, RUSTFLAGS='-D warnings', CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(root / 'default'))
            env.pop('CARGO_PROFILE_RELEASE_STRIP', None)
            # The default Cargo release profile invokes Rust's defective
            # strip=debuginfo pass even with no explicit profile in Cargo.toml.
            command = ['cargo', '+1.96.0', 'build', '--release', '--offline', '--manifest-path', str(root / 'Cargo.toml')]
            subprocess.run(command, env=env, check=True, capture_output=True)
            broken = root / 'default/release/libtc_linkedit_repro.dylib'
            with self.assertRaisesRegex(ValueError, 'misaligned LINKEDIT'):
                checker.check_linkedit(broken)
            good = []
            for target in ('aarch64-apple-darwin', 'x86_64-apple-darwin'):
                env.update(CARGO_PROFILE_RELEASE_STRIP='none', CARGO_TARGET_DIR=str(root / 'aligned'))
                subprocess.run(command + ['--target', target], env=env, check=True, capture_output=True)
                binary = root / 'aligned' / target / 'release/libtc_linkedit_repro.dylib'
                checker.check_linkedit(binary)
                good.append(binary)
            staged = root / 'libtc_linkedit_repro.dylib'
            subprocess.run(['lipo', '-create', *(str(path) for path in good), '-output', str(staged)], check=True)
            subprocess.run(['install_name_tool', '-id', '@rpath/libtc_linkedit_repro.dylib', str(staged)], check=True)
            subprocess.run(['codesign', '--force', '--sign', '-', '--options', 'runtime', '--timestamp=none', str(staged)], check=True, capture_output=True)
            checker.check_linkedit(staged)
            checker.check_runtime(staged)
            library = ctypes.CDLL(str(staged))
            library.answer.restype = ctypes.c_size_t
            self.assertNotEqual(library.answer(), 0)


if __name__ == '__main__':
    unittest.main()
