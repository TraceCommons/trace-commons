"""Exercise a freshly built CLI when TC_FEATURE_TEST_CLI is supplied."""
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
from datetime import datetime,timedelta,timezone

@unittest.skipUnless(os.environ.get("TC_FEATURE_TEST_CLI"), "requires freshly built CLI")
class CLITests(unittest.TestCase):
    def run_cli(self,*arguments,environment=None):
        return subprocess.run([os.environ["TC_FEATURE_TEST_CLI"],*arguments],env=environment,capture_output=True,text=True)
    def test_build_identity_matches_packaging_and_ignores_runtime_override(self):
        import profile
        result=self.run_cli("feature-flags","build-profile","--json")
        self.assertEqual(result.returncode,0,result.stderr)
        identity=json.loads(result.stdout)
        self.assertEqual(identity,profile.identity("production"))
        environment=dict(os.environ,TC_FEATURE_PROFILE="testing")
        overridden=self.run_cli("feature-flags","build-profile","--json",environment=environment)
        self.assertEqual(json.loads(overridden.stdout),identity)
    def test_local_snapshot_and_manifest_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            result=self.run_cli("--config-dir",directory,"feature-flags","--json")
            self.assertEqual(result.returncode,0,result.stderr)
            snapshot=json.loads(result.stdout)
            self.assertEqual(snapshot["source"],"default")
            self.assertFalse(snapshot["fresh"])
            now=datetime.now(timezone.utc)
            manifest=dict(schema_version=1,environment="production",revision=1,issued_at=now.isoformat(),expires_at=(now+timedelta(hours=1)).isoformat(),flags={})
            path=pathlib.Path(directory)/"manifest.json"
            path.write_text(json.dumps(manifest))
            valid=self.run_cli("feature-flags","validate","--file",str(path),"--environment","production")
            self.assertEqual(valid.returncode,0,valid.stderr)
            wrong=self.run_cli("feature-flags","validate","--file",str(path),"--environment","staging")
            self.assertNotEqual(wrong.returncode,0)
            manifest["flags"]={"unregistered":True};path.write_text(json.dumps(manifest))
            unknown=self.run_cli("feature-flags","validate","--file",str(path),"--environment","production")
            self.assertNotEqual(unknown.returncode,0)

if __name__=="__main__":unittest.main()
