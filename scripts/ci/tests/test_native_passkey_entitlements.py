"""Synthetic metadata fixtures; these do not establish Apple-issued grants."""
import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / 'native-passkey-entitlements.py'
spec = importlib.util.spec_from_file_location('native_passkey_entitlements', SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class NativePasskeyEntitlementsTests(unittest.TestCase):
    def setUp(self):
        self.entitlements = {
            'com.apple.application-identifier': 'KXSWJN7WY8.ai.tracecommons.shell',
            'com.apple.developer.team-identifier': 'KXSWJN7WY8',
            'keychain-access-groups': ['KXSWJN7WY8.ai.tracecommons.shell'],
        }
        self.profile = {'Entitlements': dict(self.entitlements)}

    def test_native_bundle_requires_passkeys_by_default(self):
        for required in [False, True]:
            with self.assertRaises(module.Refusal):
                module.validate(self.entitlements, self.profile, 'ai.tracecommons.shell', required)

    def test_granted_native_domain_and_wildcard(self):
        self.entitlements[module.DOMAIN_KEY] = [module.DOMAIN]
        for grant in [[module.DOMAIN], ['*'], '*']:
            self.profile['Entitlements'][module.DOMAIN_KEY] = grant
            module.validate(self.entitlements, self.profile, 'ai.tracecommons.shell', True)

    def test_requested_domain_with_old_profile_fails_even_without_flag(self):
        self.entitlements[module.DOMAIN_KEY] = [module.DOMAIN]
        with self.assertRaises(module.Refusal):
            module.validate(self.entitlements, self.profile, 'ai.tracecommons.shell', False)

    def test_wrong_signed_domain_or_grant_or_team_fails(self):
        self.entitlements[module.DOMAIN_KEY] = [module.DOMAIN]
        self.profile['Entitlements'][module.DOMAIN_KEY] = ['*']
        for target, key, value in [
            (self.entitlements, module.DOMAIN_KEY, ['webcredentials:evil.example']),
            (self.entitlements, module.DOMAIN_KEY, module.DOMAIN),
            (self.profile['Entitlements'], module.DOMAIN_KEY, ['webcredentials:evil.example']),
            (self.profile['Entitlements'], module.DOMAIN_KEY, 'webcredentials:evil.example'),
            (self.entitlements, 'com.apple.developer.team-identifier', 'OTHER'),
            (self.profile['Entitlements'], 'com.apple.developer.team-identifier', 'OTHER'),
        ]:
            with self.subTest(key=key, value=value):
                original = target[key]
                target[key] = value
                with self.assertRaises(module.Refusal):
                    module.validate(self.entitlements, self.profile, 'ai.tracecommons.shell', True)
                target[key] = original

    def test_invalid_cli_uses_a_fixed_label_without_echoing_arguments(self):
        import subprocess
        marker = 'Bearer-CLI-SECRET-invite'
        base = ['python3', str(SCRIPT), '--entitlements', 'unused',
                '--profile', 'unused', '--bundle-id', 'ai.tracecommons.shell']
        for tail in [['--' + marker], ['--entitlements']]:
            result = subprocess.run(base + tail, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stderr, 'FAIL: native-entitlement-arguments-invalid\n')
            self.assertEqual(result.stdout, '')
            self.assertNotIn(marker, result.stderr)

    def test_cli_refuses_expired_or_malformed_metadata_without_traceback(self):
        import datetime
        import plistlib
        import subprocess
        import tempfile
        self.entitlements[module.DOMAIN_KEY] = [module.DOMAIN]
        self.profile['Entitlements'][module.DOMAIN_KEY] = '*'
        self.profile['ExpirationDate'] = datetime.datetime(2020, 1, 1)
        with tempfile.TemporaryDirectory() as tmp:
            ent = pathlib.Path(tmp) / 'ent.plist'
            pro = pathlib.Path(tmp) / 'profile.plist'
            ent.write_bytes(plistlib.dumps(self.entitlements))
            for profile in [self.profile, []]:
                pro.write_bytes(plistlib.dumps(profile))
                result = subprocess.run(['python3', str(SCRIPT), '--entitlements', str(ent),
                    '--profile', str(pro), '--bundle-id', 'ai.tracecommons.shell'], capture_output=True, text=True)
                self.assertEqual(result.returncode, 1)
                self.assertIn('FAIL:', result.stderr)
                self.assertNotIn('Traceback', result.stderr)

    def test_tauri_is_not_required_to_request_native_domain(self):
        module.validate({}, {}, 'ai.tracecommons.desktop', False)
        with self.assertRaises(module.Refusal):
            module.validate({}, {}, 'ai.tracecommons.desktop', True)

    def test_profile_keychain_grant_must_be_an_exact_array_member(self):
        self.entitlements[module.DOMAIN_KEY] = [module.DOMAIN]
        self.profile['Entitlements'][module.DOMAIN_KEY] = '*'
        for grant in [['KXSWJN7WY8.*'], ['KXSWJN7WY8.ai.tracecommons.shell']]:
            self.profile['Entitlements']['keychain-access-groups'] = grant
            module.validate(self.entitlements, self.profile, 'ai.tracecommons.shell', False)
        for grant in [['OTHER.*'], ['prefix-KXSWJN7WY8.*'], 'KXSWJN7WY8.*', None]:
            self.profile['Entitlements']['keychain-access-groups'] = grant
            with self.assertRaises(module.Refusal):
                module.validate(self.entitlements, self.profile, 'ai.tracecommons.shell', False)


if __name__ == '__main__':
    unittest.main()
