import copy
import importlib.util
import json
import pathlib
import os
import plistlib
import subprocess
import tempfile
import unittest
from datetime import datetime, timezone, timedelta
ROOT = pathlib.Path(__file__).resolve().parent

def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

publish = load("publish")
profile = load("profile")
class FakeStorage:
    def __init__(self, previous=None):
        self.previous, self.writes, self.conflict = previous, [], False
    def read(self, target):
        return ("7", self.previous) if self.previous else ("0", None)
    def write(self, target, path, generation):
        if self.conflict:
            raise ValueError("generation-precondition-failed")
        self.writes.append((target, pathlib.Path(path).read_bytes(), generation))

class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.temp.name) / "manifest.json"
        now = datetime.now(timezone.utc)
        self.doc = dict(schema_version=1, environment="staging", revision=2, issued_at=now.isoformat(), expires_at=(now+timedelta(hours=1)).isoformat(), flags={})
        self.path.write_text(json.dumps(self.doc))
    def tearDown(self):
        self.temp.cleanup()
    def validator(self, path, environment):
        item = json.loads(pathlib.Path(path).read_text())
        if item["environment"] != environment or "unknown" in item["flags"]:
            raise ValueError("invalid-manifest")
        return item
    def run_publish(self, storage, apply=False):
        return publish.execute("staging", self.path, storage, self.validator, apply, commit="fixture")
    def test_dry_run_writes_nothing_and_publish_is_environment_scoped(self):
        storage=FakeStorage()
        result=self.run_publish(storage)
        self.assertEqual(storage.writes, [])
        self.assertEqual(result["environment"], "staging")
        self.run_publish(storage, True)
        self.assertEqual(len(storage.writes),1)
        self.assertEqual(storage.writes[0][0], publish.TARGETS["staging"])
        self.assertEqual(storage.writes[0][2], "0")
    def test_rejects_wrong_environment_unknown_flags_and_stale_revision(self):
        for key,value in [("environment","production"),("flags",{"unknown":True})]:
            item=copy.deepcopy(self.doc);item[key]=value;self.path.write_text(json.dumps(item))
            with self.assertRaises(ValueError):self.run_publish(FakeStorage(),True)
        self.path.write_text(json.dumps(self.doc))
        with self.assertRaises(ValueError):self.run_publish(FakeStorage(self.doc),True)
    def test_generation_conflict_does_not_overwrite(self):
        previous=copy.deepcopy(self.doc);previous["revision"]=1
        storage=FakeStorage(previous);storage.conflict=True
        with self.assertRaises(ValueError):self.run_publish(storage,True)
        self.assertEqual(storage.writes,[])
    def test_publisher_uploads_validated_copy_not_mutable_input(self):
        storage=FakeStorage()
        def mutate(path, environment):
            item=self.validator(path,environment)
            self.path.write_text("invalid replacement")
            return item
        publish.execute("staging",self.path,storage,mutate,True,commit="fixture")
        self.assertEqual(json.loads(storage.writes[0][1]),self.doc)
    def test_testing_builds_have_a_separate_update_feed(self):
        root = ROOT.parents[1]
        script = root / "macos/scripts/info-plist.sh"
        feeds = []
        for selected in ["staging", "testing"]:
            environment = dict(os.environ, TC_CHANNEL="staging", TC_FEATURE_PROFILE=selected,
                               TC_STAGING_SPARKLE_PUBLIC_ED_KEY="fixture")
            raw = subprocess.check_output(["bash", str(script), "0.0.0-test", "1"], env=environment)
            plist = plistlib.loads(raw)
            self.assertEqual(plist["TCFeatureEnvironment"], selected)
            feeds.append(plist["SUFeedURL"])
        self.assertNotEqual(*feeds)
        self.assertTrue(feeds[1].endswith("/staging/testing/appcast.xml"))

    def test_profile_agreement_and_channel_constraints(self):
        for channel,env in [("production","production"),("staging","staging"),("staging","testing")]:
            identity=profile.identity(channel,env)
            plist=dict(TCFeatureEnvironment=env,TCFeatureProfileDigest=identity["profile_digest"])
            profile.verify_values(identity,identity,identity,plist)
            bad=dict(identity,profile_digest="b"*64)
            with self.assertRaises(ValueError):profile.verify_values(identity,bad,identity,plist)
        with self.assertRaises(ValueError):profile.identity("production","testing")
        with self.assertRaises(ValueError):profile.identity("production","staging")

if __name__=="__main__":unittest.main()
