#!/usr/bin/env python3
"""Native-only Associated Domains qualification, using decoded signed metadata.

CMS decoding is not a substitute for codesign verification or a signed staging
create/login. Tests use synthetic dictionaries, never counterfeit CMS profiles.
"""
import argparse
import datetime
import pathlib
import plistlib
import subprocess
import sys

DOMAIN_KEY = 'com.apple.developer.associated-domains'
DOMAIN = 'webcredentials:tracecommons.ai'
TEAM = 'KXSWJN7WY8'
NATIVE_BUNDLE = 'ai.tracecommons.shell'


class Refusal(ValueError):
    pass


def validate(entitlements, profile, bundle_id, require_passkeys):
    if bundle_id != NATIVE_BUNDLE:
        if require_passkeys:
            raise Refusal('native passkey qualification requires the native bundle')
        return
    if not isinstance(entitlements, dict) or not isinstance(profile, dict):
        raise Refusal('unreadable native entitlement metadata')
    grant = profile.get('Entitlements')
    if not isinstance(grant, dict):
        raise Refusal('profile has no entitlement dictionary')
    for metadata in (entitlements, grant):
        if metadata.get('com.apple.application-identifier') != TEAM + '.' + NATIVE_BUNDLE:
            raise Refusal('native application identifier mismatch')
        if metadata.get('com.apple.developer.team-identifier') != TEAM:
            raise Refusal('native team identifier mismatch')
    access_group = TEAM + '.' + NATIVE_BUNDLE
    if entitlements.get('keychain-access-groups') != [access_group]:
        raise Refusal('native keychain access group mismatch')
    keychain_grants = grant.get('keychain-access-groups')
    if (not isinstance(keychain_grants, list)
            or not all(isinstance(x, str) for x in keychain_grants)
            or not any(x in (TEAM + '.*', access_group) for x in keychain_grants)):
        raise Refusal('profile does not grant native keychain access group')
    requested = entitlements.get(DOMAIN_KEY)
    if requested != [DOMAIN]:
        raise Refusal('native passkeys require exactly webcredentials:tracecommons.ai')
    allowed = grant.get(DOMAIN_KEY)
    # Apple's renewed Developer ID CMS represents the wildcard grant as a
    # scalar '*'. Signed requested entitlements must still be an exact array.
    if allowed != '*' and (not isinstance(allowed, list)
            or not all(isinstance(x, str) for x in allowed)
            or not any(x in ('*', DOMAIN) for x in allowed)):
        raise Refusal('profile does not grant native webcredentials domain')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--entitlements', type=pathlib.Path, required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--profile', type=pathlib.Path, help='decoded profile plist')
    source.add_argument('--profile-cms', type=pathlib.Path, help='real Apple CMS profile')
    parser.add_argument('--bundle-id', required=True)
    parser.add_argument('--require-passkeys', action='store_true')
    args = parser.parse_args()
    try:
        entitlements = plistlib.loads(args.entitlements.read_bytes())
        if args.profile_cms:
            decoded = subprocess.run(['security', 'cms', '-D', '-i', str(args.profile_cms)],
                                     capture_output=True, timeout=20, check=True).stdout
            profile = plistlib.loads(decoded)
        else:
            profile = plistlib.loads(args.profile.read_bytes())
        validate(entitlements, profile, args.bundle_id, args.require_passkeys)
        if args.bundle_id == NATIVE_BUNDLE:
            expiry = profile.get('ExpirationDate')
            if not isinstance(expiry, datetime.datetime):
                raise Refusal('profile has no expiration date')
            expiry = expiry.replace(tzinfo=datetime.timezone.utc)
            if expiry - datetime.datetime.now(datetime.timezone.utc) <= datetime.timedelta(days=180):
                raise Refusal('profile expires within 180 days')
        print('PASS: native entitlement metadata' + (' (passkeys required)' if args.require_passkeys else ''))
        return 0
    except Refusal as error:
        print('FAIL: ' + str(error), file=sys.stderr)
    except (OSError, ValueError, TypeError, subprocess.SubprocessError, plistlib.InvalidFileException):
        print('FAIL: unreadable native entitlement/profile metadata', file=sys.stderr)
    return 1


if __name__ == '__main__':
    sys.exit(main())
