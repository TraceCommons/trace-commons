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
        if not self.previous:
            return ("0", None)
        # The current object as served: a signed document around the manifest.
        return ("7", json.dumps(dict(payload=json.dumps(self.previous), signatures=[])).encode())

    def write(self, target, path, generation):
        if self.conflict:
            raise ValueError("generation-precondition-failed")
        self.writes.append((target, pathlib.Path(path).read_bytes(), generation))

def fake_signer(path, environment):
    return json.dumps(dict(payload=pathlib.Path(path).read_text(), signatures=[dict(key_id="0"*16, signature="0"*128)])).encode()

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
    def run_publish(self, storage, apply=False, initial=None):
        if initial is None:
            initial = storage.previous is None
        return publish.execute("staging", self.path, storage, self.validator, apply, commit="fixture", initial=initial, signer=fake_signer)
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
        publish.execute("staging",self.path,storage,mutate,True,commit="fixture",initial=True,signer=fake_signer)
        self.assertEqual(json.loads(json.loads(storage.writes[0][1])["payload"]),self.doc)
    def test_missing_object_refuses_without_initial(self):
        storage=FakeStorage()
        for apply in (False,True):
            with self.assertRaises(ValueError) as caught:
                publish.execute("staging",self.path,storage,self.validator,apply,commit="fixture",signer=fake_signer)
            message=str(caught.exception)
            self.assertIn("--initial",message)
            self.assertIn("highest accepted revision",message)
        self.assertEqual(storage.writes,[])
    def test_initial_refused_when_object_exists(self):
        previous=copy.deepcopy(self.doc);previous["revision"]=1
        storage=FakeStorage(previous)
        with self.assertRaises(ValueError) as caught:
            publish.execute("staging",self.path,storage,self.validator,True,commit="fixture",initial=True,signer=fake_signer)
        self.assertIn("--initial",str(caught.exception))
        self.assertEqual(storage.writes,[])
    def test_initial_publishes_when_object_missing(self):
        storage=FakeStorage()
        publish.execute("staging",self.path,storage,self.validator,True,commit="fixture",initial=True,signer=fake_signer)
        self.assertEqual(len(storage.writes),1)
        self.assertEqual(storage.writes[0][2],"0")
    def test_publish_uploads_only_a_signed_document(self):
        storage=FakeStorage()
        with self.assertRaises(ValueError) as caught:
            publish.execute("staging",self.path,storage,self.validator,True,commit="fixture",initial=True)
        self.assertIn("signer-required",str(caught.exception))
        self.assertEqual(storage.writes,[])
        result=self.run_publish(storage,True)
        self.assertTrue(result["signed"])
        self.assertIn("signatures",json.loads(storage.writes[0][1]))
    def test_dry_run_does_not_sign(self):
        def refuse(path,environment):raise AssertionError("dry run signed")
        result=publish.execute("staging",self.path,FakeStorage(),self.validator,False,commit="fixture",initial=True,signer=refuse)
        self.assertFalse(result["signed"])
    def test_current_object_before_signing_still_sets_the_floor(self):
        previous=copy.deepcopy(self.doc);previous["revision"]=5
        self.assertEqual(publish.current_manifest(json.dumps(previous).encode())["revision"],5)
        signed=json.dumps(dict(payload=json.dumps(previous),signatures=[])).encode()
        self.assertEqual(publish.current_manifest(signed)["revision"],5)
        with self.assertRaises(ValueError):publish.current_manifest(b'{"payload":7}')
    def test_existing_object_requires_higher_revision(self):
        previous=copy.deepcopy(self.doc);previous["revision"]=5
        self.path.write_text(json.dumps(dict(self.doc,revision=5)))
        with self.assertRaises(ValueError):self.run_publish(FakeStorage(previous),True)

class SourceCheckTests(unittest.TestCase):
    HEAD="0123456789abcdef0123456789abcdef01234567"
    def check(self,tracked=True,dirty="",ancestor=True,version=None,head=None):
        calls=[]
        def git(*args):
            calls.append(args)
            if args[0]=="rev-parse":return 0,(head or self.HEAD)+"\n"
            if args[0]=="ls-files":return (0 if tracked else 1),""
            if args[0]=="status":return 0,dirty
            if args[0]=="merge-base":return (0 if ancestor else 1),""
            raise AssertionError(args)
        if version is None:version="near-ai 0.1.0 (commit 0123456, built 2026-11-10T00:00:00Z)"
        def cli_version(binary):
            if isinstance(version,Exception):raise version
            return version
        result=publish.check_source(pathlib.Path("/repo/m.json"),pathlib.Path("/bin/near-ai"),git,cli_version)
        return result,calls
    def test_clean_state_has_no_problems(self):
        (commit,problems),calls=self.check()
        self.assertEqual(commit,self.HEAD)
        self.assertEqual(problems,[])
        self.assertIn(("merge-base","--is-ancestor","HEAD","origin/main"),calls)
    def test_untracked_modified_and_not_on_main(self):
        self.assertTrue(self.check(tracked=False)[0][1])
        self.assertTrue(self.check(dirty=" M m.json\n")[0][1])
        problems=self.check(ancestor=False)[0][1]
        self.assertTrue(any("origin/main" in item for item in problems))
    def test_cli_commit_mismatch_unreadable_or_unknown(self):
        for version in ["near-ai 0.1.0 (commit fedcba9, built x)","near-ai 0.1.0 (commit unknown, built x)","near-ai 0.1.0","near-ai 0.1.0 (commit 0123, built x)"]:
            problems=self.check(version=version)[0][1]
            self.assertTrue(any("cli" in item for item in problems),version)
        problems=self.check(version=OSError("nope"))[0][1]
        self.assertTrue(any("cli" in item for item in problems))
    def test_cli_with_channel_suffix_matches_by_prefix(self):
        version="near-ai 0.1.0 (commit 0123456, built 2026-11-10T00:00:00Z) staging"
        self.assertEqual(self.check(version=version)[0][1],[])
    def test_head_unreadable_is_an_error(self):
        def git(*args):return 1,""
        with self.assertRaises(ValueError):
            publish.check_source(pathlib.Path("/repo/m.json"),pathlib.Path("/bin/x"),git,lambda b:"")

class ProfileTests(unittest.TestCase):
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

    def test_remote_enable_guards_match_the_rust_validator(self):
        registry={"flags":{"a":{"class":"product","default":False},"p":{"class":"privacy_sensitive","default":False},"d":{"class":"product","default":True}}}
        key="ab"*32
        profile.check_remote_enables(registry,{"remote_enable":["a"],"signing_keys":[key]})
        for enables,keys in [(["a"],[]),(["p"],[key]),(["d"],[key]),(["a"],["AB"*32])]:
            with self.assertRaises(ValueError):profile.check_remote_enables(registry,{"remote_enable":enables,"signing_keys":keys})
        with self.assertRaises(ValueError):profile.check_remote_enables({"flags":{"x":{"default":False}}},{"remote_enable":[],"signing_keys":[]})

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
