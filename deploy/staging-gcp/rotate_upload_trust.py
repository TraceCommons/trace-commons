#!/usr/bin/env python3
"""Root-only bounded staging worker renewal; never prints credentials."""
import base64
import datetime
import fcntl
import grp
import hashlib
import json
import os
import pathlib
import re
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

PROJECT = 'tracecommons-pilot-2026'
VM_ID = '3442764711403503959'
SERVICE_ACCOUNT = 'tc-staging-runtime@tracecommons-pilot-2026.iam.gserviceaccount.com'
SECRET = 'tc-staging-account-trust-worker-token'
FEATURE = pathlib.Path('/etc/tracecommons/client-upload-trust.env')
TOKEN_KEY = 'TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN'
LOCK = pathlib.Path('/run/tc-upload-trust-rotation/lock')
ISSUER = 'https://issuer.staging.tracecommons.ai'
AUDIENCE = 'trace-commons-ingest'

class RotationError(Exception):
    """Only static, nonsecret controls belong in this exception."""

def require(ok, label):
    if not ok:
        raise RotationError(label)

def load_issuer_private_key(pem):
    """Load Ed25519 PKCS8, including the issuer's exact RFC5958 v2 encoding.

    Older OpenSSL/cryptography reject OneAsymmetricKey with its public-key
    field. The fallback accepts only its 83-byte canonical Ed25519 shape;
    it never implements a permissive/general ASN.1 parser.
    """
    require(isinstance(pem, bytes) and len(pem) <= 4096, 'issuer_key_format_invalid')
    lines = pem.strip().splitlines()
    require(len(lines) >= 3 and lines[0] == b'-----BEGIN PRIVATE KEY-----' and lines[-1] == b'-----END PRIVATE KEY-----', 'issuer_key_format_invalid')
    body = b''.join(lines[1:-1])
    require(all(re.fullmatch(rb'[A-Za-z0-9+/]+={0,2}', line) for line in lines[1:-1]), 'issuer_key_format_invalid')
    try:
        der = base64.b64decode(body, validate=True)
    except Exception:
        raise RotationError('issuer_key_format_invalid') from None
    require(base64.b64encode(der) == body and 2 <= len(der) <= 4096 and der[0] == 0x30, 'issuer_key_format_invalid')
    # Check the whole canonical DER envelope even on the library path so a
    # parser that tolerates trailing data cannot silently accept it.
    if der[1] < 0x80:
        header_length, content_length = 2, der[1]
    else:
        require(der[1] in [0x81, 0x82], 'issuer_key_format_invalid')
        count = der[1] & 0x7f
        header_length = 2+count
        require(len(der) >= header_length and der[2] != 0, 'issuer_key_format_invalid')
        content_length = int.from_bytes(der[2:header_length], 'big')
        require(content_length >= 128 and (content_length.bit_length()+7)//8 == count, 'issuer_key_format_invalid')
    require(header_length+content_length == len(der), 'issuer_key_format_invalid')
    try:
        private = serialization.load_pem_private_key(pem, password=None)
    except Exception:
        private = None
    if private is not None:
        require(isinstance(private, Ed25519PrivateKey), 'issuer_key_algorithm_invalid')
        return private
    # SEQUENCE(81), INTEGER(version=1), SEQUENCE(OID 1.3.101.112),
    # OCTET STRING(34 containing OCTET STRING(32 seed)), [1](33: 0 unused bits
    # followed by exactly 32 public-key bytes). No attributes/trailing fields.
    require(len(der) == 83 and der[:16] == bytes.fromhex('3051020101300506032b657004220420') and der[48:51] == bytes.fromhex('812100'), 'issuer_key_format_invalid')
    private = Ed25519PrivateKey.from_private_bytes(der[16:48])
    public = private.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    require(public == der[51:83], 'issuer_key_public_mismatch')
    return private

def parse_env(text, strict=True):
    result = {}
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        match = re.match(r'^([A-Z_][A-Z0-9_]*)=(.*)$', line)
        if not match:
            require(not strict, 'feature_line_invalid')
            continue
        key, value = match.groups()
        require(not strict or key not in result, 'feature_duplicate_key')
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ['"', "'"]:
            value = value[1:-1]
        result[key] = value
    return result

def validate_feature(text):
    values = parse_env(text)
    policy_prefixes = [prefix for prefix in ['TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_', 'TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_'] if prefix + 'JSON' in values]
    require(len(policy_prefixes) == 1, 'feature_policy_mode_invalid')
    prefix = policy_prefixes[0]
    expected = {prefix+'JSON', prefix+'VERSION', 'TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL', TOKEN_KEY, 'TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED', 'TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS', 'TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT', 'TRACE_COMMONS_ADMISSION_ENABLED', 'TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED'}
    require(set(values) == expected, 'feature_key_set_invalid')
    require(values['TRACE_COMMONS_ADMISSION_ENABLED'] == values['TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED'] == 'false', 'admission_must_remain_disabled')
    require(values['TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED'] == 'true', 'scheduler_disabled')
    require(values['TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS'] == '300' and values['TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT'] == '10000', 'scheduler_bounds_changed')
    policy = json.loads(values[prefix+'JSON'])
    require(values[prefix+'VERSION'] == policy.get('version') == 'staging-earned-upload-v1' and policy.get('growth_rule') == 'tiered-v1', 'policy_target_changed')
    db = urllib.parse.urlsplit(values['TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL'])
    require(db.scheme == 'postgres' and db.username == 'tc_upload_trust_login' and db.hostname == '127.0.0.1' and db.port == 5432 and db.path == '/tracecommons' and not db.query and not db.fragment, 'database_target_changed')
    return values

def b64(data):
    return base64.urlsafe_b64encode(data).rstrip(b'=').decode()

def unb64(data):
    require(bool(re.fullmatch(r'[A-Za-z0-9_-]+', data)), 'worker_encoding_invalid')
    return base64.urlsafe_b64decode(data+'='*((-len(data)) % 4))

def verify_token(token, context, now):
    try:
        header_part, payload_part, signature = token.split('.')
        header = json.loads(unb64(header_part))
        require(header.get('alg') == 'EdDSA' and header.get('typ') == 'JWT', 'worker_algorithm_invalid')
        public = context['public_keys'].get(header.get('kid'))
        require(public is not None, 'worker_managed_key_missing')
        public.verify(unb64(signature), (header_part+'.'+payload_part).encode())
        claims = json.loads(unb64(payload_part))
        require(set(claims) == {'tenant_id', 'sub', 'role', 'iss', 'aud', 'iat', 'exp', 'jti'}, 'worker_claim_set_invalid')
        require(claims['role'] == 'account_trust_worker' and claims['tenant_id'] == 'staging-upload-trust' and claims['sub'] == 'staging-upload-trust-worker', 'worker_scope_invalid')
        require(claims['iss'] == ISSUER and claims['aud'] == AUDIENCE, 'worker_target_invalid')
        require(isinstance(claims['iat'], int) and not isinstance(claims['iat'], bool) and isinstance(claims['exp'], int) and not isinstance(claims['exp'], bool), 'worker_time_invalid')
        require(claims['iat'] <= now+60 and 0 < claims['exp']-claims['iat'] <= min(604800, context['max_ttl']), 'worker_ttl_invalid')
        require(isinstance(claims['jti'], str) and bool(claims['jti'].strip()), 'worker_jti_invalid')
        return claims
    except RotationError:
        raise
    except Exception:
        raise RotationError('worker_signature_or_claims_invalid') from None

def mint(context, now):
    ttl = min(604800, context['max_ttl'])
    # A daily timer needs a full day between its 48h renewal threshold and expiry.
    require(ttl >= 259200, 'worker_ttl_too_short_for_daily_renewal')
    claims = {'tenant_id':'staging-upload-trust', 'sub':'staging-upload-trust-worker', 'role':'account_trust_worker', 'iss':ISSUER, 'aud':AUDIENCE, 'iat':now, 'exp':now+ttl, 'jti':str(uuid.uuid4())}
    message = b64(json.dumps({'alg':'EdDSA', 'typ':'JWT', 'kid':context['kid']}, separators=(',', ':')).encode())+'.'+b64(json.dumps(claims, separators=(',', ':')).encode())
    token = message+'.'+b64(context['private_key'].sign(message.encode()))
    verify_token(token, context, now)
    return token, claims

def replace_token(text, token):
    changed, count = re.subn(r'^'+TOKEN_KEY+r'=.*$', TOKEN_KEY+'='+token, text, flags=re.MULTILINE)
    require(count == 1, 'feature_token_line_count_invalid')
    return changed

def rotate(ops, now):
    previous = ops.feature_text()
    if previous is None:
        return {'status':'disabled'}
    ops.verify_identity()
    values = validate_feature(previous)
    context = ops.context()
    old_claims = verify_token(values[TOKEN_KEY], context, now)
    if old_claims['exp']-now > 172800:
        return {'status':'not_due', 'expiry_utc':datetime.datetime.fromtimestamp(old_claims['exp'], datetime.timezone.utc).isoformat()}
    previous_secret, _ = ops.latest_token()
    previous_secret_claims = verify_token(previous_secret, context, now)
    token, claims = mint(context, now)
    candidate = replace_token(previous, token)
    digest = ops.main_digest()
    try:
        version = ops.add_token(token)
        ops.write_feature(candidate)
        ops.restart()
        ops.verify(validate_feature(candidate), token)
        require(ops.main_digest() == digest, 'main_configuration_changed')
    except Exception:
        # Never replace a usable new credential with an expired previous one.
        if old_claims['exp'] > now+300:
            errors = []
            for restore in [lambda:ops.write_feature(previous), lambda:ops.add_token(previous_secret if previous_secret_claims['exp'] > now+300 else values[TOKEN_KEY]), ops.restart, lambda:ops.verify(values, values[TOKEN_KEY])]:
                try:
                    restore()
                except Exception:
                    errors.append(True)
            raise RotationError('rotation_failed_rollback_incomplete' if errors else 'rotation_failed_rollback_complete') from None
        raise RotationError('rotation_failed_no_valid_previous_credential') from None
    return {'status':'rotated', 'secret_version':version, 'expiry_utc':datetime.datetime.fromtimestamp(claims['exp'], datetime.timezone.utc).isoformat(), 'jti_sha256':hashlib.sha256(claims['jti'].encode()).hexdigest()}

class Operations:
    def __init__(self):
        self.snapshot = None
        self.metadata_token = None
        self.previous_commit = None

    def safe_read(self, path, missing=False):
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        except FileNotFoundError:
            if missing:
                return None
            raise RotationError('configuration_file_missing') from None
        with os.fdopen(fd, 'r') as handle:
            info = os.fstat(handle.fileno())
            require(stat.S_ISREG(info.st_mode) and info.st_uid == 0 and not info.st_mode & 0o027, 'configuration_permissions_invalid')
            return handle.read()

    def feature_text(self):
        self.snapshot = self.safe_read(FEATURE, missing=True)
        return self.snapshot

    def metadata(self, path):
        request = urllib.request.Request('http://metadata.google.internal/computeMetadata/v1/'+path, headers={'Metadata-Flavor':'Google'})
        try:
            return urllib.request.urlopen(request, timeout=10).read().decode()
        except Exception:
            raise RotationError('metadata_unavailable') from None

    def verify_identity(self):
        require(self.metadata('project/project-id') == PROJECT and self.metadata('instance/id') == VM_ID and self.metadata('instance/service-accounts/default/email') == SERVICE_ACCOUNT, 'staging_identity_mismatch')

    def process_env(self, service):
        try:
            pid = self.command(['systemctl', 'show', service, '-p', 'MainPID', '--value'])
            if not pid.isdigit() or int(pid) <= 1:
                return None
            return dict(item.split('=', 1) for item in pathlib.Path('/proc/'+pid+'/environ').read_text().split('\0') if '=' in item)
        except (FileNotFoundError, RotationError):
            return None

    def context(self):
        main = parse_env(self.safe_read('/etc/tracecommons/ingest.env'), strict=False)
        issuer = parse_env(self.safe_read('/etc/tracecommons/issuer.env'), strict=False)
        ingest_runtime = self.process_env('trace-commons-ingest')
        issuer_runtime = self.process_env('trace-commons-upload-claim-issuer')
        if ingest_runtime:
            main = ingest_runtime
        if issuer_runtime:
            issuer = issuer_runtime
        require(main.get('TRACE_COMMONS_SIGNED_TOKEN_ISSUER') == ISSUER and main.get('TRACE_COMMONS_SIGNED_TOKEN_AUDIENCE') == AUDIENCE, 'ingest_token_target_changed')
        require(main.get('TRACE_COMMONS_ADMISSION_ENABLED', 'false') == main.get('TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED', 'false') == 'false', 'admission_must_remain_disabled')
        require(issuer.get('TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER') == ISSUER and issuer.get('TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE') == AUDIENCE, 'issuer_token_target_changed')
        key_path = issuer.get('TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_FILE')
        require(key_path == '/etc/tracecommons/issuer-signing-v1.pem', 'issuer_key_path_changed')
        private = load_issuer_private_key(self.safe_read(key_path).encode())
        require(isinstance(private, Ed25519PrivateKey), 'issuer_key_algorithm_invalid')
        keyset_url = main['TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL']
        url = urllib.parse.urlsplit(keyset_url)
        require(url.scheme == 'https' and url.hostname == 'issuer.staging.tracecommons.ai' and url.port in [None, 443] and not url.username and not url.password and not url.query and not url.fragment, 'managed_keyset_target_changed')
        try:
            with urllib.request.urlopen(keyset_url, timeout=15) as response:
                keyset = json.load(response)
        except Exception:
            raise RotationError('managed_keyset_unavailable') from None
        keys = {}
        now = datetime.datetime.now(datetime.timezone.utc)
        for key in keyset['keys']:
            if any(field in key and key[field] is not None and (datetime.datetime.fromisoformat(key[field].replace('Z', '+00:00')) > now if field == 'not_before' else datetime.datetime.fromisoformat(key[field].replace('Z', '+00:00')) <= now) for field in ['not_before', 'not_after']):
                continue
            keys[key['kid']] = serialization.load_pem_public_key(key['public_key_pem'].encode())
        kid = issuer['TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID']
        require(kid in keys, 'current_issuer_key_not_managed')
        public_format = (serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        require(private.public_key().public_bytes(*public_format) == keys[kid].public_bytes(*public_format), 'current_issuer_key_mismatch')
        maximum = int(main.get('TRACE_COMMONS_SIGNED_TOKEN_MAX_TTL_SECONDS', '604800'))
        require(maximum > 0, 'verifier_ttl_invalid')
        self.previous_commit = self.health().get('build_commit')
        return {'kid':kid, 'private_key':private, 'public_keys':keys, 'issuer':ISSUER, 'audience':AUDIENCE, 'max_ttl':maximum}

    def secret_request(self, operation, payload=None):
        require(operation in ['access', 'addVersion'], 'secret_operation_invalid')
        if self.metadata_token is None:
            self.metadata_token = json.loads(self.metadata('instance/service-accounts/default/token'))['access_token']
        resource = 'https://secretmanager.googleapis.com/v1/projects/'+PROJECT+'/secrets/'+SECRET
        url = resource+'/versions/latest:access' if operation == 'access' else resource+':addVersion'
        body = None if operation == 'access' else json.dumps({'payload':{'data':base64.b64encode(payload.encode()).decode()}}, separators=(',', ':')).encode()
        request = urllib.request.Request(url, data=body, method='GET' if operation == 'access' else 'POST', headers={'Authorization':'Bearer '+self.metadata_token, 'Content-Type':'application/json'})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RotationError('secret_manager_http_'+str(error.code)) from None
        except Exception:
            raise RotationError('secret_manager_unavailable') from None

    def latest_token(self):
        result = self.secret_request('access')
        return base64.b64decode(result['payload']['data']).decode(), result['name'].rsplit('/', 1)[-1]

    def add_token(self, token):
        result = self.secret_request('addVersion', token)
        version = result['name'].rsplit('/', 1)[-1]
        require(version.isdigit(), 'secret_version_response_invalid')
        return version

    def main_digest(self):
        return hashlib.sha256(self.safe_read('/etc/tracecommons/ingest.env').encode()).hexdigest()

    def write_feature(self, text):
        require(self.safe_read(FEATURE) == self.snapshot, 'feature_changed_concurrently')
        fd, temporary = tempfile.mkstemp(prefix='.client-upload-trust-', dir=str(FEATURE.parent))
        try:
            os.fchmod(fd, 0o640)
            os.fchown(fd, 0, grp.getgrnam('tracecommons').gr_gid)
            with os.fdopen(fd, 'w') as handle:
                handle.write(text)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, FEATURE)
            self.snapshot = text
            directory = os.open(FEATURE.parent, os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)

    def command(self, args):
        process = subprocess.run(args, capture_output=True, text=True, timeout=90)
        require(process.returncode == 0, 'command_failed_'+pathlib.Path(args[0]).name)
        return process.stdout.strip()

    def restart(self):
        self.command(['systemctl', 'restart', 'trace-commons-ingest'])

    def health(self):
        try:
            with urllib.request.urlopen('http://127.0.0.1:3907/health', timeout=3) as response:
                return json.load(response)
        except Exception:
            return {}

    def verify(self, expected, token):
        for _ in range(30):
            health = self.health()
            runtime = self.process_env('trace-commons-ingest')
            if health.get('near_ai_staging') is True and runtime and all(runtime.get(key) == value for key, value in expected.items()):
                require(not self.previous_commit or health.get('build_commit') == self.previous_commit, 'running_build_changed')
                require(runtime.get('TRACE_COMMONS_ADMISSION_ENABLED', 'false') == runtime.get('TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED', 'false') == 'false', 'running_admission_enabled')
                # Startup already checks both trust readiness contracts; exercise
                # the authenticated narrow DB-backed worker seam as well.
                mode = 'applied' if 'TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON' in expected else 'shadow'
                request = urllib.request.Request('http://127.0.0.1:3907/v1/admin/evaluate-account-trust?mode='+mode+'&limit=1', data=b'', method='POST', headers={'Authorization':'Bearer '+token})
                try:
                    with urllib.request.urlopen(request, timeout=20) as response:
                        summary = json.load(response)
                        require(response.status == 200 and isinstance(summary, dict) and summary.get('mode') == mode and type(summary.get('failed')) is int and summary['failed'] == 0, 'worker_readiness_failed')
                    return
                except Exception:
                    raise RotationError('worker_readiness_failed') from None
            time.sleep(1)
        raise RotationError('running_feature_verification_failed')

def main():
    require(os.geteuid() == 0, 'root_required')
    LOCK.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    require(LOCK.parent.stat().st_uid == 0 and stat.S_IMODE(LOCK.parent.stat().st_mode) == 0o700, 'lock_directory_permissions_invalid')
    with open(LOCK, 'a') as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        result = rotate(Operations(), int(time.time()))
    print(json.dumps(result, sort_keys=True))

if __name__ == '__main__':
    try:
        main()
    except RotationError as error:
        print(json.dumps({'status':'failed', 'control':str(error)}))
        raise SystemExit(1)
    except Exception as error:
        print(json.dumps({'status':'failed', 'control':'unexpected_'+type(error).__name__}))
        raise SystemExit(1)
