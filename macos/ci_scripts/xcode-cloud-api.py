#!/usr/bin/env python3
"""Start an Xcode Cloud pilot build, wait for it, and fetch its archive.

Run from GitHub Actions (`.github/workflows/xcode-cloud-pilot.yml`). It uses
only the standard library and the system `openssl`: no third-party App Store
Connect client runs in a job that holds the Developer ID key.

Credentials come from the environment, never argv:
  ASC_KEY_P8_PATH   path to the App Store Connect API private key (.p8)
  ASC_KEY_ID        its key ID
  ASC_ISSUER_ID     the team's issuer ID

  xcode-cloud-api.py start   --workflow ID --branch NAME   -> prints the run ID
  xcode-cloud-api.py wait    --run ID [--timeout-minutes N] -> prints run JSON
  xcode-cloud-api.py archive --run ID --out FILE.zip       -> downloads it

`wait` returns once the run is complete, whatever its result. While Apple's
managed Developer ID certificate cannot be created (HTTP 409, resultCode 7460;
see docs/operator/xcode-cloud-pilot.md), every run ends FAILED at the
Developer ID export even though the archive succeeded. `archive` therefore
requires the Archive action's ARCHIVE artifact, not a successful run.
"""

import argparse
import base64
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

API = 'https://api.appstoreconnect.apple.com'


def b64url(data):
    return base64.urlsafe_b64encode(data).rstrip(b'=').decode()


def der_to_raw(der):
    """openssl writes an ECDSA signature as DER; JWS ES256 wants r || s."""
    if der[0] != 0x30:
        raise ValueError('not a DER sequence')
    i = 2 if der[1] < 0x80 else 2 + (der[1] & 0x7f)
    out = b''
    for _ in range(2):
        if der[i] != 0x02:
            raise ValueError('not a DER integer')
        n = der[i + 1]
        value = der[i + 2:i + 2 + n].lstrip(b'\x00')
        if len(value) > 32:
            raise ValueError('integer longer than 32 bytes')
        out += value.rjust(32, b'\x00')
        i += 2 + n
    return out


def token():
    now = int(time.time())
    header = {'alg': 'ES256', 'kid': os.environ['ASC_KEY_ID'], 'typ': 'JWT'}
    claims = {'iss': os.environ['ASC_ISSUER_ID'], 'iat': now, 'exp': now + 1200,
              'aud': 'appstoreconnect-v1'}
    signing_input = b64url(json.dumps(header, separators=(',', ':')).encode()) + '.' + \
        b64url(json.dumps(claims, separators=(',', ':')).encode())
    der = subprocess.run(
        ['openssl', 'dgst', '-sha256', '-sign', os.environ['ASC_KEY_P8_PATH']],
        input=signing_input.encode(), capture_output=True, check=True).stdout
    return signing_input + '.' + b64url(der_to_raw(der))


def call(method, path, body=None):
    url = path if path.startswith('https://') else API + path
    data = json.dumps(body).encode() if body is not None else None
    for attempt in range(5):
        request = urllib.request.Request(url, data=data, method=method, headers={
            'Authorization': 'Bearer ' + token(), 'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.loads(response.read() or b'{}')
        except urllib.error.HTTPError as error:
            if error.code in (429, 500, 502, 503, 504) and attempt < 4:
                time.sleep(10 * (attempt + 1))
                continue
            sys.exit(f'{method} {url}: HTTP {error.code}: {error.read().decode(errors="replace")}')
        except urllib.error.URLError as error:
            if attempt < 4:
                time.sleep(10 * (attempt + 1))
                continue
            sys.exit(f'{method} {url}: {error}')


def pages(path):
    while path:
        page = call('GET', path)
        yield from page.get('data', [])
        path = page.get('links', {}).get('next')


def branch_reference(workflow, branch):
    repository = call('GET', f'/v1/ciWorkflows/{workflow}/repository')['data']['id']
    query = urllib.parse.urlencode({'limit': 200})
    for reference in pages(f'/v1/scmRepositories/{repository}/gitReferences?{query}'):
        attributes = reference['attributes']
        if attributes.get('kind') == 'BRANCH' and attributes.get('name') == branch \
                and not attributes.get('isDeleted'):
            return reference['id']
    sys.exit(f'branch {branch!r} is not known to Xcode Cloud yet; push it and retry')


def start(args):
    reference = branch_reference(args.workflow, args.branch)
    run = call('POST', '/v1/ciBuildRuns', {'data': {
        'type': 'ciBuildRuns',
        'attributes': {'clean': True},
        'relationships': {
            'workflow': {'data': {'type': 'ciWorkflows', 'id': args.workflow}},
            'sourceBranchOrTag': {'data': {'type': 'scmGitReferences', 'id': reference}},
        },
    }})
    print(run['data']['id'])


def wait(args):
    deadline = time.time() + args.timeout_minutes * 60
    while True:
        run = call('GET', f'/v1/ciBuildRuns/{args.run}')['data']
        attributes = run['attributes']
        print(f"build {attributes.get('number')}: {attributes.get('executionProgress')} "
              f"{attributes.get('completionStatus') or ''}", file=sys.stderr)
        if attributes.get('executionProgress') == 'COMPLETE':
            print(json.dumps(attributes))
            return
        if time.time() > deadline:
            sys.exit(f'build run {args.run} did not complete in {args.timeout_minutes} minutes')
        time.sleep(30)


def archive(args):
    for action in pages(f'/v1/ciBuildRuns/{args.run}/actions'):
        if action['attributes'].get('actionType') != 'ARCHIVE':
            continue
        for artifact in pages(f"/v1/ciBuildActions/{action['id']}/artifacts"):
            attributes = artifact['attributes']
            if attributes.get('fileType') == 'ARCHIVE' and attributes.get('downloadUrl'):
                # A pre-signed download URL: it must not carry the API token.
                urllib.request.urlretrieve(attributes['downloadUrl'], args.out)
                print(attributes.get('fileName'), file=sys.stderr)
                return
    sys.exit(f'build run {args.run} produced no archive')


def main():
    for name in ('ASC_KEY_P8_PATH', 'ASC_KEY_ID', 'ASC_ISSUER_ID'):
        if not os.environ.get(name):
            sys.exit(f'{name} is not set')
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest='command', required=True)
    p = commands.add_parser('start')
    p.add_argument('--workflow', required=True)
    p.add_argument('--branch', required=True)
    p.set_defaults(func=start)
    p = commands.add_parser('wait')
    p.add_argument('--run', required=True)
    p.add_argument('--timeout-minutes', type=int, default=60)
    p.set_defaults(func=wait)
    p = commands.add_parser('archive')
    p.add_argument('--run', required=True)
    p.add_argument('--out', required=True)
    p.set_defaults(func=archive)
    args = parser.parse_args()
    args.func(args)


if __name__ == '__main__':
    main()
