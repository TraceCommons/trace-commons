import base64,copy,json,pathlib,tempfile,unittest
import rotate_upload_trust as r
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

NOW=1800000000
KEY=Ed25519PrivateKey.generate()
def fixture_token(exp,role='account_trust_worker'):
 head=base64.urlsafe_b64encode(b'{"alg":"EdDSA","typ":"JWT","kid":"fixture"}').rstrip(b'=').decode()
 claims={'tenant_id':'staging-upload-trust','sub':'staging-upload-trust-worker','role':role,'iss':'https://issuer.staging.tracecommons.ai','aud':'trace-commons-ingest','iat':exp-604800,'exp':exp,'jti':'fixture-token-id'}
 payload=base64.urlsafe_b64encode(json.dumps(claims,separators=(',',':')).encode()).rstrip(b'=').decode()
 msg=head+'.'+payload
 return msg+'.'+base64.urlsafe_b64encode(KEY.sign(msg.encode())).rstrip(b'=').decode()
def feature(token):
 policy={'version':'staging-earned-upload-v1','growth_rule':'tiered-v1'}
 return '# retained comment\nTRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON='+json.dumps(policy,separators=(',',':'))+'\nTRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_VERSION=staging-earned-upload-v1\nTRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL=postgres://tc_upload_trust_login:aaaaaaaaaaaaaaaa@127.0.0.1:5432/tracecommons\nTRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED=true\nTRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS=300\nTRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT=10000\nTRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN='+token+'\nTRACE_COMMONS_ADMISSION_ENABLED=false\nTRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED=false\n'
class FakeOps:
 def __init__(self,text):
  self.text=text;self.main='unchanged-main';self.versions=[];self.restarts=0;self.fail_restart_once=False;self.fail_verify_once=False;self.bad_host=False
  self.ctx={'kid':'fixture','private_key':KEY,'public_keys':{'fixture':KEY.public_key()},'issuer':'https://issuer.staging.tracecommons.ai','audience':'trace-commons-ingest','max_ttl':604800}
  self.latest=fixture_token(NOW+86400)
 def feature_text(self):return self.text
 def verify_identity(self):
  if self.bad_host:raise r.RotationError('staging_identity_mismatch')
 def context(self):return self.ctx
 def latest_token(self):return self.latest,'1'
 def add_token(self,token):self.versions.append(token);self.latest=token;return str(len(self.versions)+1)
 def main_digest(self):return self.main
 def write_feature(self,text):self.text=text
 def restart(self):
  self.restarts+=1
  if self.fail_restart_once:self.fail_restart_once=False;raise r.RotationError('restart_failed')
 def verify(self,expected,token):
  if self.fail_verify_once:self.fail_verify_once=False;raise r.RotationError('readiness_failed')
class RotationTests(unittest.TestCase):
 def test_disabled_never_touches_remote_dependencies(self):
  ops=FakeOps(None);ops.bad_host=True
  self.assertEqual(r.rotate(ops,NOW),{'status':'disabled'})
  self.assertEqual(ops.restarts,0);self.assertEqual(ops.versions,[])
 def test_not_due_preserves_entire_file(self):
  original=feature(fixture_token(NOW+172801));ops=FakeOps(original)
  self.assertEqual(r.rotate(ops,NOW)['status'],'not_due')
  self.assertEqual(ops.text,original);self.assertEqual(ops.versions,[]);self.assertEqual(ops.restarts,0)
 def test_due_rotates_only_token_with_bounded_worker_scope(self):
  old=fixture_token(NOW+172800);original=feature(old);ops=FakeOps(original)
  result=r.rotate(ops,NOW)
  self.assertEqual(result['status'],'rotated');self.assertEqual(ops.restarts,1)
  new=ops.versions[0];claims=json.loads(base64.urlsafe_b64decode(new.split('.')[1]+'=='))
  self.assertEqual(claims['exp'],NOW+604800);self.assertEqual(claims['iat'],NOW)
  self.assertEqual(claims['role'],'account_trust_worker');self.assertEqual(claims['tenant_id'],'staging-upload-trust')
  self.assertEqual(ops.text,original.replace(old,new));self.assertNotIn(new,json.dumps(result));self.assertNotIn(claims['jti'],json.dumps(result))
 def test_restart_failure_restores_valid_file_and_secret(self):
  original=feature(fixture_token(NOW+86400));ops=FakeOps(original);previous_secret=ops.latest;ops.fail_restart_once=True
  with self.assertRaises(r.RotationError):r.rotate(ops,NOW)
  self.assertEqual(ops.text,original);self.assertEqual(ops.latest,previous_secret);self.assertEqual(ops.restarts,2)
 def test_readiness_failure_restores_valid_file_and_secret(self):
  original=feature(fixture_token(NOW+86400));ops=FakeOps(original);ops.fail_verify_once=True;previous_secret=ops.latest
  with self.assertRaises(r.RotationError):r.rotate(ops,NOW)
  self.assertEqual(ops.text,original);self.assertEqual(ops.latest,previous_secret)
 def test_expired_credential_can_recover_but_never_rollback_to_expired(self):
  original=feature(fixture_token(NOW-1));ops=FakeOps(original);ops.latest=fixture_token(NOW-1);ops.fail_verify_once=True
  with self.assertRaises(r.RotationError):r.rotate(ops,NOW)
  self.assertNotEqual(ops.text,original);self.assertEqual(len(ops.versions),1)
 def test_admin_credential_cannot_be_renewed(self):
  original=feature(fixture_token(NOW+86400,role='admin'));ops=FakeOps(original)
  with self.assertRaises(r.RotationError):r.rotate(ops,NOW)
  self.assertEqual(ops.text,original);self.assertEqual(ops.versions,[])
 def test_wrong_vm_cannot_mutate_credentials(self):
  original=feature(fixture_token(NOW+86400));ops=FakeOps(original);ops.bad_host=True
  with self.assertRaises(r.RotationError):r.rotate(ops,NOW)
  self.assertEqual(ops.text,original);self.assertEqual(ops.versions,[])
 def test_configured_verifier_ttl_is_respected(self):
  ops=FakeOps(feature(fixture_token(NOW+86400)));ops.ctx['max_ttl']=172800
  # Existing seven-day token is outside the now-configured verifier policy.
  with self.assertRaises(r.RotationError):r.rotate(ops,NOW)
  self.assertEqual(ops.versions,[])
 def test_duplicate_token_and_enabled_admission_refuse(self):
  for text in [feature(fixture_token(NOW+86400))+'TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN=duplicate\n',feature(fixture_token(NOW+86400)).replace('TRACE_COMMONS_ADMISSION_ENABLED=false','TRACE_COMMONS_ADMISSION_ENABLED=true')]:
   ops=FakeOps(text)
   with self.assertRaises(r.RotationError):r.rotate(ops,NOW)
   self.assertEqual(ops.versions,[])

class SecretBoundaryTests(unittest.TestCase):
 def test_secret_transport_has_only_worker_secret_access_and_version_add(self):
  from unittest.mock import patch
  class Response:
   def __enter__(self):return self
   def __exit__(self,*args):pass
   def read(self):return b'{"name":"projects/tracecommons-pilot-2026/secrets/tc-staging-account-trust-worker-token/versions/9"}'
  ops=r.Operations();ops.metadata_token='fixture-access-token';requests=[]
  def transport(request,timeout):requests.append(request);return Response()
  with patch.object(r.urllib.request,'urlopen',transport):
   ops.secret_request('access');ops.secret_request('addVersion','fixture-worker-token')
   with self.assertRaises(r.RotationError):ops.secret_request('setIamPolicy')
  self.assertEqual([q.full_url for q in requests],['https://secretmanager.googleapis.com/v1/projects/tracecommons-pilot-2026/secrets/tc-staging-account-trust-worker-token/versions/latest:access','https://secretmanager.googleapis.com/v1/projects/tracecommons-pilot-2026/secrets/tc-staging-account-trust-worker-token:addVersion'])
  self.assertEqual([q.method for q in requests],['GET','POST'])
  payload=json.loads(requests[1].data);self.assertEqual(base64.b64decode(payload['payload']['data']),b'fixture-worker-token')
 def test_live_helper_atomic_write_preserves_file_mode_and_rejects_conflict(self):
  from unittest.mock import patch
  from types import SimpleNamespace
  import os
  with tempfile.TemporaryDirectory()as directory:
   target=pathlib.Path(directory)/'feature.env';target.write_text('old\n')
   ops=r.Operations();ops.snapshot='old\n';ops.safe_read=lambda path,missing=False:pathlib.Path(path).read_text()
   with patch.object(r,'FEATURE',target),patch.object(r.grp,'getgrnam',lambda name:SimpleNamespace(gr_gid=os.getgid())),patch.object(r.os,'fchown',lambda *args:None):
    ops.write_feature('new\n');self.assertEqual(target.read_text(),'new\n');self.assertEqual(target.stat().st_mode&0o777,0o640)
    target.write_text('concurrent\n')
    with self.assertRaises(r.RotationError):ops.write_feature('lost-update\n')
    self.assertEqual(target.read_text(),'concurrent\n')

class WorkerVerificationTests(unittest.TestCase):
 def run_verification(self,mode,failed):
  from unittest.mock import patch
  class Response:
   status=200
   def __enter__(self):return self
   def __exit__(self,*args):pass
   def read(self):return json.dumps({'mode':mode,'accounts_scanned':1,'accounts_evaluated':1 if failed==0 else 0,'failed':failed,'tier_changed':0}).encode()
  ops=r.Operations();expected=r.validate_feature(feature(fixture_token(NOW+86400)))
  if mode=='shadow':
   expected={key.replace('TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_','TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_'):value for key,value in expected.items()}
  ops.health=lambda:{'near_ai_staging':True,'build_commit':'fixture-build'};ops.process_env=lambda service:dict(expected);ops.previous_commit='fixture-build';requests=[]
  def transport(request,timeout):requests.append(request);return Response()
  with patch.object(r.urllib.request,'urlopen',transport):ops.verify(expected,expected[r.TOKEN_KEY])
  return requests
 def test_applied_verification_requests_applied_policy(self):
  requests=self.run_verification('applied',0)
  self.assertEqual(requests[0].full_url,'http://127.0.0.1:3907/v1/admin/evaluate-account-trust?mode=applied&limit=1')
 def test_shadow_verification_requests_shadow_policy(self):
  requests=self.run_verification('shadow',0)
  self.assertEqual(requests[0].full_url,'http://127.0.0.1:3907/v1/admin/evaluate-account-trust?mode=shadow&limit=1')
 def test_http_200_with_failed_evaluation_does_not_pass_readiness(self):
  with self.assertRaises(r.RotationError):self.run_verification('applied',1)



class IssuerKeyFormatTests(unittest.TestCase):
 def setUp(self):
  from cryptography.hazmat.primitives import serialization
  self.serialization=serialization
  self.seed=bytes(range(32))
  self.key=Ed25519PrivateKey.from_private_bytes(self.seed)
  self.public=self.key.public_key().public_bytes(serialization.Encoding.Raw,serialization.PublicFormat.Raw)
  self.der=bytes.fromhex('3051020101300506032b657004220420')+self.seed+bytes.fromhex('812100')+self.public
 def pem(self,der):
  return b'-----BEGIN PRIVATE KEY-----\n'+base64.b64encode(der)+b'\n-----END PRIVATE KEY-----\n'
 def test_standard_and_rfc5958_v2_sign_identically(self):
  normal=self.key.private_bytes(self.serialization.Encoding.PEM,self.serialization.PrivateFormat.PKCS8,self.serialization.NoEncryption())
  one=r.load_issuer_private_key(normal);two=r.load_issuer_private_key(self.pem(self.der))
  self.assertIsInstance(one,Ed25519PrivateKey);self.assertIsInstance(two,Ed25519PrivateKey)
  message=b'offline issuer key format regression'
  self.assertEqual(one.sign(message),two.sign(message));self.key.public_key().verify(two.sign(message),message)
 def test_rfc5958_v2_rejects_malformed_der_and_public_mismatch(self):
  mutations=[(0,0x31),(1,0x50),(2,0x03),(4,0x00),(11,0x71),(12,0x05),(13,0x21),(14,0x05),(15,0x1f),(48,0xa1),(49,0x20),(50,0x01),(51,self.der[51]^1)]
  for offset,value in mutations:
   der=bytearray(self.der);der[offset]=value
   with self.subTest(offset=offset),self.assertRaises(r.RotationError):r.load_issuer_private_key(self.pem(bytes(der)))
  for der in [self.der+b'\x00',self.der[:-1],self.der[:16]+self.seed[:-1]+self.der[48:]]:
   with self.subTest(length=len(der)),self.assertRaises(r.RotationError):r.load_issuer_private_key(self.pem(der))
 def test_rejects_invalid_base64_wrong_pem_label_and_trailing_material(self):
  valid=self.pem(self.der)
  for pem in [valid.replace(b'PRIVATE KEY',b'EC PRIVATE KEY'),valid.replace(b'MFEC',b'!FEC'),valid+b'extra',valid+valid,b'x'*4097]:
   with self.subTest(size=len(pem)),self.assertRaises(r.RotationError):r.load_issuer_private_key(pem)
 def test_standard_pkcs8_rejects_appended_der_bytes(self):
  der=self.key.private_bytes(self.serialization.Encoding.DER,self.serialization.PrivateFormat.PKCS8,self.serialization.NoEncryption())
  with self.assertRaises(r.RotationError):r.load_issuer_private_key(self.pem(der+b'\x00'))

if __name__=='__main__':unittest.main()
