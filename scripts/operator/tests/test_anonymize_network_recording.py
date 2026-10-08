"""Adversarial synthetic fixtures, not captured pilot responses."""
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / 'anonymize-network-recording.py'
spec = importlib.util.spec_from_file_location('anonymizer', SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
SECRET = 'Bearer secret-invite /Users/private/session https://token.example/private'
PROOF = {key: 0 for key in module.PROOF_KEYS}
PROOF['pending'] = 1
TOTAL = {'calls': 1, 'priced_calls': 0, 'cost_usd': 0.0, 'proof': PROOF}
ZERO = {'calls': 0, 'priced_calls': 0, 'cost_usd': 0.0,
        'proof': {key: 0 for key in module.PROOF_KEYS}}


class AnonymizerTests(unittest.TestCase):
    def project(self, surface, body):
        result = module.record(surface, body, 'synthetic', '2026-10-02', 200)
        encoded = json.dumps(result)
        self.assertNotIn(SECRET, encoded)
        self.assertNotIn('/Users/', encoded)
        self.assertNotIn('https:', encoded)
        self.assertEqual(result['source_kind'], 'synthetic')
        self.assertEqual(result['representation'], 'allowlisted_projection')
        return result['payload']

    def test_invite_name_is_removed_and_estimate_preserved(self):
        payload = self.project('invite_lookup', {'valid': True,
            'issuer_display_name': SECRET,
            'credit_range': {'min': 1, 'max': 5, 'unit': 'points_per_accepted_trace'}})
        self.assertEqual(payload['credit_range']['unit'], 'points_per_accepted_trace')
        self.assertNotIn('issuer_display_name', payload)

    def test_unknown_source_fields_and_versions_refused(self):
        for surface, body in [('invite_lookup', {'valid': True, 'token': SECRET}),
                              ('mission_catalog', {'schema_version': 2, 'entries': [], 'next_cursor': None})]:
            with self.assertRaises(module.Refusal):
                self.project(surface, body)

    def test_summary_identifiers_and_source_timestamp_removed(self):
        group = dict(TOTAL, model=SECRET, backend=SECRET, route='routed', work_kind=None)
        payload = self.project('ironwire_summary', {'enabled': True, 'receipts': True,
            'since': '2026-10-01T00:00:00Z', 'groups': [group],
            'routed': TOTAL, 'outside': ZERO, 'unknown': ZERO})
        self.assertEqual(payload['groups'][0]['proof']['verified'], 0)
        self.assertEqual(payload['groups'][0]['proof']['pending'], 1)
        self.assertNotIn('since', payload)
        self.assertNotIn('model', payload['groups'][0])

    def test_summary_invalid_counts_and_unknown_work_classification_refused(self):
        for change in [{'priced_calls': 2}, {'calls': -1}, {'cost_usd': float('nan')},
                       {'work_kind': SECRET}, {'calls': True}, {'cost_usd': 10 ** 400}]:
            group = dict(TOTAL, model=SECRET, backend=SECRET, route='routed', work_kind=None)
            group.update(change)
            with self.assertRaises(module.Refusal):
                self.project('ironwire_summary', {'enabled': True, 'receipts': True,
                    'since': '2026-10-01T00:00:00Z', 'groups': [group],
                    'routed': TOTAL, 'outside': ZERO, 'unknown': ZERO})

    def test_log_known_secret_bearing_fields_are_removed(self):
        row = {'id': 414, 'started_at': SECRET, 'facade': SECRET, 'path': SECRET,
               'conversation': SECRET, 'client_session_id': SECRET, 'backend': SECRET,
               'requested_model': SECRET, 'served_model': SECRET, 'upstream_id': SECRET,
               'body_ref': SECRET, 'request_sha256': SECRET, 'response_sha256': SECRET,
               'error': SECRET, 'rung': SECRET, 'confidence': {'secret': SECRET},
               'status': 200, 'cost_usd': None, 'input_tokens': None,
               'cache_read_tokens': 0, 'cache_write_tokens': 0, 'output_tokens': 0,
               'ttfb_ms': None, 'total_ms': 10, 'attempts': 1, 'substitutions': None,
               'proof': 'pending'}
        payload = self.project('ironwire_log', {'enabled': True, 'exchanges': [row]})
        self.assertIsNone(payload['exchanges'][0]['cost_usd'])
        self.assertIsNone(payload['exchanges'][0]['input_tokens'])
        self.assertNotIn('id', payload['exchanges'][0])

    def test_credit_never_relays_currency_for_disabled_or_ungraded(self):
        for settlement, graded in [('disabled', False), ('dry_run', True), ('http', False)]:
            payload = self.project('credit_summary', {'points': {'earned_this_period': 0,
                'lifetime_earned': 1}, 'pending_review': 2,
                'posture': {'settlement': settlement, 'graded': graded, 'explanation': SECRET},
                'currency': {'code': SECRET, 'earned_this_period': SECRET},
                'period': {'start': SECRET, 'end': SECRET}})
            self.assertIsNone(payload['currency'])
            self.assertEqual(payload['pending_review'], 2)
        payload = self.project('settlement_posture', {'settlement': 'disabled', 'graded': False,
                                                     'explanation': SECRET})
        self.assertEqual(payload, {'settlement': 'disabled', 'graded': False})

    def test_mission_projection_discards_hashes_ids_preview_and_dates(self):
        payload = self.project('mission_catalog', {'schema_version': 1, 'entries': [{
            'mission_id': SECRET, 'program_id': SECRET, 'package_sha256': SECRET,
            'offer_version_hash': SECRET, 'task_preview': SECRET, 'published_at': SECRET}],
            'next_cursor': SECRET})
        self.assertEqual(payload, {'schema_version': 1, 'entry_count': 1, 'has_next_page': True})

    def test_credit_ipc_preserves_unknown_halves_as_null(self):
        value = {key: None for key in module.CREDIT_IPC_KEYS}
        value.update(posture_state='unknown', points_state='unknown', observed_at=SECRET)
        payload = self.project('commons_credit_ipc', value)
        self.assertEqual(payload['posture_state'], 'unknown')
        self.assertIsNone(payload['commons_graded'])
        self.assertIsNone(payload['commons_pending_review'])
        value['commons_pending_review'] = 0
        with self.assertRaises(module.Refusal):
            self.project('commons_credit_ipc', value)

    def test_credit_ipc_refusal_is_not_an_http_status_or_measured_zero(self):
        payload = self.project('commons_credit_ipc', {'id': 1,
            'error': {'code': 'unknown_method', 'message': SECRET}})
        self.assertEqual(payload, {'readable': False, 'ipc_error': 'unknown_method'})

    def test_failure_body_is_never_interpreted_as_success(self):
        result = module.record('mission_catalog', SECRET, 'authorized_recording', '2026-10-02', 503)
        self.assertEqual(result['payload'], {'readable': False, 'http_status': 503})
        self.assertNotIn(SECRET, json.dumps(result))

    def test_committed_projection_digests_and_provenance_are_explicit(self):
        import hashlib
        root = SCRIPT.parent / 'fixtures' / 'network-recordings'
        for path in root.glob('*.json'):
            fixture = json.loads(path.read_text())
            self.assertEqual(fixture['representation'], 'allowlisted_projection')
            self.assertIn(fixture['source_kind'], ('synthetic', 'authorized_recording'))
            payload = json.dumps(fixture['payload'], sort_keys=True, separators=(',', ':'), allow_nan=False).encode()
            self.assertEqual(fixture['payload_sha256'], hashlib.sha256(payload).hexdigest())
        fake = json.loads((root / 'ironwire-fake-local-projection.json').read_text())
        self.assertEqual(fake['source_kind'], 'synthetic')

    def test_invalid_cli_never_echoes_secret_values_or_unknown_flags(self):
        marker = 'Bearer-CLI-SECRET-invite'
        base = ['python3', str(SCRIPT), '--input', 'unused', '--output', 'unused',
                '--surface', 'invite_lookup', '--source-kind', 'synthetic',
                '--capture-date', '2026-10-02']
        for tail in [['--surface', marker], ['--source-kind', marker],
                     ['--http-status', marker], ['--' + marker]]:
            result = subprocess.run(base + tail, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stderr, 'FAIL: recording-refused\n')
            self.assertEqual(result.stdout, '')
            self.assertNotIn(marker, result.stderr)

    def test_no_output_on_refusal_or_missing_authorized_source_ack(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'input.json'
            output = pathlib.Path(tmp) / 'output.json'
            source.write_text(json.dumps({'valid': True, 'secret': SECRET}))
            base = ['python3', str(SCRIPT), '--input', str(source), '--output', str(output),
                    '--surface', 'invite_lookup', '--capture-date', '2026-10-02']
            for tail in [['--source-kind', 'synthetic'], ['--source-kind', 'authorized_recording']]:
                result = subprocess.run(base + tail, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists())
                self.assertNotIn(SECRET, result.stdout + result.stderr)
                self.assertNotIn(str(source), result.stdout + result.stderr)

    def test_duplicate_keys_deep_json_and_nonfinite_numbers_refused(self):
        for data in [b'{"valid":true,"valid":false}', b'{"x":NaN}',
                     b'[' * 100 + b'0' + b']' * 100]:
            with self.assertRaises(module.Refusal):
                module.decode(data)


if __name__ == '__main__':
    unittest.main()
