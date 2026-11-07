#!/usr/bin/python3
"""Local integration tests: public-key gates and real universal Mach-O fixtures."""
import base64
import json
import importlib.util
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[2]


class PilotTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='tc-cloud-pilot-')
        cls.repo = Path(cls.temp.name) / 'repo'
        for directory in ('macos/scripts', 'macos/ci_scripts', 'macos/channels', 'crates/trace-commons-contributor'):
            (cls.repo / directory).mkdir(parents=True)
        for name in ('prepare-xcode-cloud.sh', 'info-plist.sh', 'info-plist-strings.sh', 'channel.sh'):
            shutil.copy2(SOURCE / 'macos/scripts' / name, cls.repo / 'macos/scripts' / name)
        shutil.copy2(SOURCE / 'macos/ci_scripts/check-pilot-archive.py', cls.repo / 'macos/ci_scripts')
        shutil.copy2(SOURCE / 'macos/channels/production.env', cls.repo / 'macos/channels')
        (cls.repo / 'crates/trace-commons-contributor/Cargo.toml').write_text('[package]\nversion = "1.2.3"\n')
        subprocess.run(['git', 'init', '-q', str(cls.repo)], check=True)
        subprocess.run(['git', '-C', str(cls.repo), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(cls.repo), '-c', 'user.name=Pilot Test', '-c', 'user.email=pilot@example.invalid', 'commit', '-qm', 'Fixture'], check=True)
        cls.env = dict(os.environ, TC_CHANNEL='production', TC_SPARKLE_PUBLIC_ED_KEY=base64.b64encode(bytes(32)).decode(),
                       TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX='11' * 32, CI_BUILD_NUMBER='7')
        cls.env.pop('CI_COMMIT', None)
        cls.prepare()
        cls.generated = cls.repo / 'macos/.xcode-cloud/generated'
        cls.metadata = json.loads((cls.generated / 'pilot-build.json').read_text())
        cls.archive = Path(cls.temp.name) / 'valid.xcarchive'
        app = cls.archive / 'Products/Applications/Near.ai.app'
        resources = app / 'Contents/Resources'
        framework = app / 'Contents/Frameworks/Sparkle.framework'
        (app / 'Contents/MacOS').mkdir(parents=True)
        resources.mkdir()
        framework.mkdir(parents=True)
        (cls.archive / 'Info.plist').write_bytes(plistlib.dumps({'ApplicationProperties': {'ApplicationPath': 'Applications/Near.ai.app'}}))
        shutil.copy2(cls.generated / 'Info.plist', app / 'Contents/Info.plist')
        shutil.copytree(cls.generated / 'en.lproj', resources / 'en.lproj')
        metadata = dict(cls.metadata, mode='release-universal')
        (resources / 'pilot-build.json').write_text(json.dumps(metadata))
        (cls.generated / 'pilot-build.json').write_text(json.dumps(metadata))
        for resource in ('AppIcon.icns', 'Assets.car'):
            (resources / resource).write_bytes(b'fixture')
        (resources / 'TraceCommons_TCShellCore.bundle').mkdir()
        for nested in ('Versions/B/Updater.app', 'Versions/B/XPCServices/Downloader.xpc', 'Versions/B/XPCServices/Installer.xpc'):
            (framework / nested).mkdir(parents=True)
        ffi_source = Path(cls.temp.name) / 'ffi.c'
        ffi_source.write_text('#include <stdlib.h>\n#include <string.h>\nchar *tc_build_channel_json(void){return strdup("{\\"channel\\":\\"production\\"}");}\nvoid tc_string_free(char *p){free(p);}\n')
        dylib = app / 'Contents/Frameworks/libtrace_commons_contributor_ffi.dylib'
        subprocess.run(['xcrun', 'clang', '-arch', 'arm64', '-arch', 'x86_64', '-dynamiclib', str(ffi_source), '-install_name', '@rpath/' + dylib.name, '-o', str(dylib)], check=True)
        cli_source = Path(cls.temp.name) / 'cli.c'
        line = 'near-ai 1.2.3 (commit ' + metadata['sourceCommit'] + ', built 2026-11-07T00:00:00Z) channel production'
        cli_source.write_text('#include <stdio.h>\nint main(void){puts(' + json.dumps(line) + ');return 0;}\n')
        cli = app / 'Contents/MacOS/near-ai'
        subprocess.run(['xcrun', 'clang', '-arch', 'arm64', '-arch', 'x86_64', str(cli_source), '-o', str(cli)], check=True)
        subprocess.run(['xcrun', 'clang', '-arch', 'arm64', '-arch', 'x86_64', str(cli_source), '-L' + str(dylib.parent), '-ltrace_commons_contributor_ffi', '-Wl,-rpath,@executable_path/../Frameworks', '-o', str(app / 'Contents/MacOS/TraceCommonsApp')], check=True)
        for binary in (dylib, cli):
            subprocess.run(['codesign', '--force', '--sign', '-', '--options', 'runtime', '--timestamp=none', str(binary)], check=True, capture_output=True)
        shutil.copy2(dylib, framework / 'Sparkle')
        shutil.copy2(cli, framework / 'Versions/B/Autoupdate')

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    @classmethod
    def prepare(cls, overrides=None):
        return subprocess.run([str(cls.repo / 'macos/scripts/prepare-xcode-cloud.sh'), '--metadata-only'],
                              env=dict(cls.env, **(overrides or {})), text=True, capture_output=True, check=not overrides)

    def verify(self, archive):
        return subprocess.run(['/usr/bin/python3', str(self.repo / 'macos/ci_scripts/check-pilot-archive.py'), str(archive), str(self.repo)], env=self.env, text=True, capture_output=True)

    def test_missing_or_malformed_keys_and_staging_fail_closed(self):
        for override in ({'TC_SPARKLE_PUBLIC_ED_KEY': ''}, {'TC_SPARKLE_PUBLIC_ED_KEY': '!invalid'},
                         {'TC_SPARKLE_PUBLIC_ED_KEY': base64.b64encode(bytes(31)).decode()},
                         {'TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX': ''}, {'CI_BUILD_NUMBER': '0'}, {'TC_CHANNEL': 'staging'}):
            with self.subTest(override=next(iter(override))):
                result = self.prepare(override)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('refusing:', result.stderr)

    def test_metadata_and_plist_contract(self):
        self.assertEqual(self.metadata['schemaVersion'], 1)
        self.assertEqual(self.metadata['mode'], 'metadata-only')
        self.assertEqual(self.metadata['architectures'], ['arm64', 'x86_64'])
        self.assertEqual(self.metadata['shortVersion'], '1.2.3')
        self.assertEqual(self.metadata['buildVersion'], '7')
        self.assertFalse((self.repo / 'macos/.xcode-cloud/ffi-universal').exists())

    def test_staged_runtime_check_rejects_plain_ad_hoc_cli(self):
        binary = Path(self.temp.name) / 'staged-near-ai'
        shutil.copy2(self.archive / 'Products/Applications/Near.ai.app/Contents/MacOS/near-ai', binary)
        command = ['/usr/bin/python3', str(self.repo / 'macos/ci_scripts/check-pilot-archive.py'), '--check-runtime', str(binary)]
        good = subprocess.run(command, text=True, capture_output=True)
        self.assertEqual(good.returncode, 0, good.stderr)
        subprocess.run(['codesign', '--force', '--sign', '-', '--options', '0', '--timestamp=none', str(binary)], check=True, capture_output=True)
        bad = subprocess.run(command, text=True, capture_output=True)
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn('missing hardened runtime', bad.stderr)

    def test_captured_bundled_cli_identity_contract(self):
        spec = importlib.util.spec_from_file_location('pilot_archive', SOURCE / 'macos/ci_scripts/check-pilot-archive.py')
        checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(checker)
        commit = '72f71eff5762194e78bdcf19e6a6dcc6cab3e557'
        captured = 'near-ai 0.24.13 (commit ' + commit + ', built 2026-11-07T22:56:04Z) channel production'
        checker.check_cli_identity(captured, '0.24.13', commit)
        for invalid in (captured.replace('near-ai ', 'trace-commons-contributor ', 1),
                        captured.replace('channel production', 'channel staging'),
                        captured.replace(commit, '0' * 40),
                        captured.replace('0.24.13', '0.24.12'),
                        captured.replace('2026-11-07T22:56:04Z', 'unknown')):
            with self.subTest(invalid=invalid):
                with self.assertRaises(ValueError):
                    checker.check_cli_identity(invalid, '0.24.13', commit)

    def test_real_universal_archive_fixture(self):
        result = self.verify(self.archive)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_archive_rejects_channel_missing_resources_and_thin_binary(self):
        for mutation in ('channel', 'resource', 'architecture', 'source', 'mode', 'runtime'):
            with self.subTest(mutation=mutation):
                archive = Path(self.temp.name) / (mutation + '.xcarchive')
                shutil.copytree(self.archive, archive)
                app = archive / 'Products/Applications/Near.ai.app'
                if mutation == 'channel':
                    path = app / 'Contents/Info.plist'
                    plist = plistlib.loads(path.read_bytes())
                    plist['TCChannel'] = 'staging'
                    path.write_bytes(plistlib.dumps(plist))
                elif mutation == 'resource':
                    shutil.rmtree(app / 'Contents/Resources/TraceCommons_TCShellCore.bundle')
                elif mutation == 'architecture':
                    binary = app / 'Contents/MacOS/near-ai'
                    subprocess.run(['lipo', str(binary), '-thin', 'arm64', '-output', str(binary) + '.thin'], check=True)
                    Path(str(binary) + '.thin').replace(binary)
                elif mutation == 'runtime':
                    subprocess.run(['codesign', '--force', '--sign', '-', '--options', '0', '--timestamp=none', str(app / 'Contents/MacOS/near-ai')], check=True, capture_output=True)
                else:
                    path = app / 'Contents/Resources/pilot-build.json'
                    metadata = json.loads(path.read_text())
                    metadata['sourceCommit' if mutation == 'source' else 'mode'] = 'invalid'
                    path.write_text(json.dumps(metadata))
                result = self.verify(archive)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn('refusing:', result.stderr)


if __name__ == '__main__':
    unittest.main()
