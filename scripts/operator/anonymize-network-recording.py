#!/usr/bin/env python3
"""Offline, explicit-input network recording projection. No credentials or I/O discovery.

Outputs are deliberately NOT wire-response replay fixtures. All identifiers,
source times, labels, prose, URLs, bodies and hashes are discarded. Unknown
fields/shapes fail closed. Real captures require an explicit authorization
acknowledgment; that metadata records the caller's assertion, not consent proof.
"""
import argparse
import datetime
import hashlib
import json
import math
import os
import pathlib
import re
import sys

MAX_BYTES = 512 * 1024
MAX_ROWS = 1000
MAX_INTEGER = (1 << 53) - 1
PROOF_KEYS = ('verified', 'gateway_only', 'unattested', 'pending', 'unavailable',
              'failed', 'outside', 'unrecorded')
SURFACES = ('invite_lookup', 'ironwire_log', 'ironwire_summary', 'credit_summary',
            'settlement_posture', 'mission_catalog', 'commons_credit_ipc')


class Refusal(ValueError):
    pass


def refuse():
    raise Refusal('unsupported-or-invalid-response')


def bounded(value, depth=0, budget=None):
    if budget is None:
        budget = [20000]
    budget[0] -= 1
    if depth > 16 or budget[0] < 0:
        refuse()
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str) or len(key) > 128:
                refuse()
            bounded(item, depth + 1, budget)
    elif isinstance(value, list):
        if len(value) > MAX_ROWS:
            refuse()
        for item in value:
            bounded(item, depth + 1, budget)
    elif isinstance(value, str):
        if len(value) > 16000:
            refuse()
    elif type(value) is int:
        if abs(value) > MAX_INTEGER:
            refuse()
    elif isinstance(value, float):
        if not math.isfinite(value):
            refuse()
    elif value is not None and type(value) not in (bool, int):
        refuse()


def decode(raw):
    if len(raw) > MAX_BYTES:
        refuse()
    def pairs(items):
        result = {}
        for key, item in items:
            if key in result:
                refuse()
            result[key] = item
        return result
    try:
        value = json.loads(raw, object_pairs_hook=pairs, parse_constant=lambda _: refuse())
        bounded(value)
        return value
    except (ValueError, UnicodeError, RecursionError):
        refuse()


def shape(value, required, optional=()):
    if not isinstance(value, dict) or not set(required) <= value.keys():
        refuse()
    if value.keys() - set(required) - set(optional):
        refuse()
    return value


def integer(value, nullable=False, maximum=MAX_INTEGER):
    if value is None and nullable:
        return None
    if type(value) is not int or not 0 <= value <= maximum:
        refuse()
    return value


def number(value, nullable=False):
    if value is None and nullable:
        return None
    if type(value) not in (int, float) or not math.isfinite(value) or not 0 <= value <= 1e12:
        refuse()
    return value


def boolean(value):
    if type(value) is not bool:
        refuse()
    return value


def label(value, allowed):
    if value not in allowed or not isinstance(value, str):
        refuse()
    return value


def discarded_strings(value, keys):
    for key in keys:
        item = value.get(key)
        if item is not None and not isinstance(item, str):
            refuse()


def posture(value):
    shape(value, ('settlement', 'graded', 'explanation'))
    discarded_strings(value, ('explanation',))
    return {'settlement': label(value['settlement'], ('http', 'dry_run', 'disabled')),
            'graded': boolean(value['graded'])}


def invite(value):
    shape(value, ('valid',), ('issuer_display_name', 'credit_range', 'reason_label'))
    discarded_strings(value, ('issuer_display_name', 'reason_label'))
    result = {'valid': boolean(value['valid'])}
    if result['valid']:
        if value.get('reason_label') is not None:
            refuse()
        credit = value.get('credit_range')
        if credit is not None:
            shape(credit, ('min', 'max', 'unit'))
            low, high = integer(credit['min']), integer(credit['max'])
            if low > high:
                refuse()
            result['credit_range'] = {'min': low, 'max': high,
                'unit': label(credit['unit'], ('points_per_accepted_trace',))}
    else:
        if value.get('credit_range') is not None or value.get('issuer_display_name') is not None:
            refuse()
        result['reason_label'] = label(value.get('reason_label'),
            ('malformed', 'not_found', 'expired', 'exhausted', 'revoked'))
    return result


def proof_counts(value):
    shape(value, PROOF_KEYS)
    return {key: integer(value[key]) for key in PROOF_KEYS}


def total(value, group=False):
    required = ('calls', 'priced_calls', 'cost_usd', 'proof')
    if group:
        required += ('model', 'backend', 'route', 'work_kind')
    shape(value, required)
    result = {key: integer(value[key]) for key in ('calls', 'priced_calls')}
    result['cost_usd'] = number(value['cost_usd'])
    result['proof'] = proof_counts(value['proof'])
    if result['priced_calls'] > result['calls'] or sum(result['proof'].values()) != result['calls']:
        refuse()
    if group:
        discarded_strings(value, ('model', 'backend'))
        if value['work_kind'] is not None:
            refuse()
        result['route'] = label(value['route'], ('routed', 'outside', 'unknown'))
    return result


def summary(value):
    shape(value, ('enabled', 'receipts', 'since', 'groups', 'routed', 'outside', 'unknown'))
    discarded_strings(value, ('since',))
    if not isinstance(value['groups'], list) or len(value['groups']) > MAX_ROWS:
        refuse()
    result = {'enabled': boolean(value['enabled']), 'receipts': boolean(value['receipts']),
              'groups': [total(group, True) for group in value['groups']]}
    for route in ('routed', 'outside', 'unknown'):
        result[route] = total(value[route])
        groups = [group for group in result['groups'] if group['route'] == route]
        for key in ('calls', 'priced_calls'):
            if sum(group[key] for group in groups) != result[route][key]:
                refuse()
        for key in PROOF_KEYS:
            if sum(group['proof'][key] for group in groups) != result[route]['proof'][key]:
                refuse()
        if not math.isclose(sum(group['cost_usd'] for group in groups), result[route]['cost_usd'],
                            rel_tol=1e-9, abs_tol=1e-9):
            refuse()
    result['cost_basis'] = 'catalog_price_not_billed_spend'
    return result


def legacy_summary(value):
    counts = ('exchanges', 'without_usage', 'input_tokens', 'cache_read_tokens',
              'cache_write_tokens', 'output_tokens', 'cold_starts')
    shape(value, counts + ('cost_usd', 'by_backend', 'cache_hit_rate',
                          'cache_by_backend', 'cost_by_backend'))
    result = {key: integer(value[key]) for key in counts}
    result['cost_usd'] = number(value['cost_usd'])
    rate = number(value['cache_hit_rate'], True)
    if rate is not None and rate > 1:
        refuse()
    result['cache_hit_rate'] = rate
    return result


def log(value):
    shape(value, ('enabled', 'exchanges'), ('last_24h',))
    if not isinstance(value['exchanges'], list) or len(value['exchanges']) > MAX_ROWS:
        refuse()
    result = {'enabled': boolean(value['enabled']), 'exchanges': [],
              'cost_basis': 'catalog_price_not_billed_spend'}
    safe_counts = ('input_tokens', 'cache_read_tokens', 'cache_write_tokens', 'output_tokens',
                   'ttfb_ms', 'total_ms', 'attempts', 'substitutions')
    discarded = ('id', 'started_at', 'facade', 'path', 'conversation', 'client_session_id',
                 'backend', 'requested_model', 'served_model', 'upstream_id', 'model_alias_resolved',
                 'request_sha256', 'response_sha256', 'body_ref', 'rung', 'error', 'confidence')
    for row in value['exchanges']:
        shape(row, ('status', 'cost_usd'), safe_counts + discarded + ('proof',))
        discarded_strings(row, tuple(key for key in discarded if key not in ('id', 'confidence')))
        if row.get('id') is not None:
            integer(row['id'])
        status = integer(row['status'], maximum=599)
        if status != 0 and status < 100:
            refuse()
        clean = {'status': status, 'cost_usd': number(row['cost_usd'], True)}
        for key in safe_counts:
            if key in row:
                clean[key] = integer(row[key], True)
        clean['proof'] = row.get('proof') if row.get('proof') in PROOF_KEYS else 'unrecorded'
        result['exchanges'].append(clean)
    if 'last_24h' in value:
        result['last_24h'] = legacy_summary(value['last_24h'])
    return result


def credit(value):
    shape(value, ('points', 'posture', 'pending_review'), ('currency', 'period'))
    shape(value['points'], ('earned_this_period', 'lifetime_earned'))
    result = {'points': {key: integer(item) for key, item in value['points'].items()},
              'pending_review': integer(value['pending_review']), 'posture': posture(value['posture']),
              'currency': None}
    if value.get('period') is not None:
        shape(value['period'], ('start', 'end'))
        discarded_strings(value['period'], ('start', 'end'))
    currency = value.get('currency')
    if currency is not None:
        shape(currency, ('code', 'earned_this_period'))
        discarded_strings(currency, ('code', 'earned_this_period'))
        if result['posture'] == {'settlement': 'http', 'graded': True}:
            # Currency is still not a payout promise. Only bounded exact USD
            # decimal strings can survive, never arbitrary currency/prose.
            amount = currency['earned_this_period']
            if currency['code'] != 'USD' or not isinstance(amount, str) or not re.fullmatch(r'(0|[1-9][0-9]{0,11})(\.[0-9]{1,6})?', amount):
                refuse()
            result['currency'] = {'code': 'USD', 'earned_this_period': amount}
    return result


def missions(value):
    shape(value, ('schema_version', 'entries', 'next_cursor'))
    if type(value['schema_version']) is not int or value['schema_version'] != 1:
        refuse()
    if not isinstance(value['entries'], list) or len(value['entries']) > 50:
        refuse()
    for entry in value['entries']:
        shape(entry, ('mission_id', 'program_id', 'package_sha256', 'offer_version_hash',
                      'task_preview', 'published_at'))
        discarded_strings(entry, tuple(entry))
    discarded_strings(value, ('next_cursor',))
    # Do not rewrite package contents while retaining their public digest.
    # This is a measured count projection, explicitly not a mission package.
    return {'schema_version': 1, 'entry_count': len(value['entries']),
            'has_next_page': value['next_cursor'] is not None}


CREDIT_IPC_KEYS = (
    'posture_state', 'commons_settlement', 'commons_settlement_explanation', 'commons_graded',
    'points_state', 'commons_points_earned_this_period', 'commons_points_lifetime_earned',
    'commons_pending_review', 'commons_currency_code', 'commons_currency_earned_this_period',
    'commons_period_start', 'commons_period_end', 'observed_at',
)


def credit_ipc(value):
    # This projection is explicitly identified as IPC, never presented as a
    # recording of an HTTP response that the capture tool did not observe.
    if isinstance(value, dict) and 'error' in value:
        shape(value, ('error',), ('id',))
        if 'id' in value:
            integer(value['id'])
        shape(value['error'], ('code', 'message'))
        discarded_strings(value['error'], ('message',))
        return {'readable': False, 'ipc_error': label(value['error']['code'],
            ('unknown_method', 'unavailable', 'not_authorized', 'bad_params', 'busy'))}
    shape(value, CREDIT_IPC_KEYS)
    discarded_strings(value, ('commons_settlement_explanation', 'commons_period_start',
                             'commons_period_end', 'observed_at'))
    result = {key: label(value[key], ('known', 'unknown'))
              for key in ('posture_state', 'points_state')}
    for key in ('commons_settlement', 'commons_graded', 'commons_points_earned_this_period',
                'commons_points_lifetime_earned', 'commons_pending_review',
                'commons_currency_code', 'commons_currency_earned_this_period'):
        result[key] = None
    if result['posture_state'] == 'known':
        result['commons_settlement'] = label(value['commons_settlement'], ('http', 'dry_run', 'disabled'))
        result['commons_graded'] = boolean(value['commons_graded'])
    elif any(value[key] is not None for key in
             ('commons_settlement', 'commons_settlement_explanation', 'commons_graded')):
        refuse()
    point_keys = ('commons_points_earned_this_period', 'commons_points_lifetime_earned', 'commons_pending_review')
    if result['points_state'] == 'known':
        for key in point_keys:
            result[key] = integer(value[key])
    elif any(value[key] is not None for key in point_keys +
             ('commons_currency_code', 'commons_currency_earned_this_period', 'commons_period_start', 'commons_period_end')):
        refuse()
    if result['points_state'] == 'known' and result['commons_graded'] is True and result['commons_settlement'] == 'http':
        currency = value['commons_currency_earned_this_period']
        code = value['commons_currency_code']
        if currency is not None or code is not None:
            if code != 'USD' or not isinstance(currency, str) or not re.fullmatch(r'(0|[1-9][0-9]{0,11})(\.[0-9]{1,6})?', currency):
                refuse()
            result['commons_currency_code'] = code
            result['commons_currency_earned_this_period'] = currency
    return result


PROJECTORS = dict(zip(SURFACES, (invite, log, summary, credit, posture, missions, credit_ipc)))


def record(surface, value, source_kind, capture_date, http_status):
    if surface not in PROJECTORS or source_kind not in ('synthetic', 'authorized_recording'):
        refuse()
    try:
        if datetime.date.fromisoformat(capture_date).isoformat() != capture_date:
            refuse()
    except (ValueError, TypeError):
        refuse()
    integer(http_status, maximum=599)
    if http_status < 100:
        refuse()
    if http_status != 200:
        payload = {'readable': False, 'http_status': http_status}
    else:
        bounded(value)
        payload = PROJECTORS[surface](value)
    content = json.dumps(payload, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()
    return {'schema_version': 'trace_commons.anonymized_network_projection.v1',
            'source_surface': surface, 'source_kind': source_kind,
            'capture_date': capture_date, 'representation': 'allowlisted_projection',
            'payload_sha256': hashlib.sha256(content).hexdigest(), 'payload': payload}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--surface', choices=SURFACES, required=True)
    parser.add_argument('--source-kind', choices=('synthetic', 'authorized_recording'), required=True)
    parser.add_argument('--authorized-source', action='store_true')
    parser.add_argument('--capture-date', required=True, help='YYYY-MM-DD, no exact source timestamps')
    parser.add_argument('--http-status', type=int, default=200)
    args = parser.parse_args()
    try:
        if args.source_kind == 'authorized_recording' and not args.authorized_source:
            refuse()
        with args.input.open('rb') as source:
            raw = source.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            refuse()
        value = decode(raw) if args.http_status == 200 else None
        result = record(args.surface, value, args.source_kind, args.capture_date, args.http_status)
        encoded = (json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + '\n').encode()
        # Exclusive creation refuses clobber and symlinks. No file is opened
        # until the complete response projection has passed validation.
        fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'wb') as output:
            output.write(encoded)
        print('PASS: ' + args.surface + ' ' + args.source_kind + ' sha256=' + result['payload_sha256'])
        return 0
    except (Refusal, OSError, ValueError, TypeError, RecursionError):
        print('FAIL: recording-refused', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
