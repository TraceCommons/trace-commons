import importlib.util
import json
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


publish = load("publish")


class FakeStorage:
    def __init__(self, previous=None):
        self.previous, self.writes, self.conflict = previous, [], False

    def read(self, target):
        if self.previous is None:
            return "0", None
        return "7", json.dumps(dict(payload=json.dumps(self.previous), signatures=[])).encode()

    def write(self, target, path, generation):
        if self.conflict:
            raise ValueError("generation-precondition-failed")
        self.writes.append((target, pathlib.Path(path).read_bytes(), generation))


def fake_signer(path, environment):
    return json.dumps(dict(payload=pathlib.Path(path).read_text(), signatures=[dict(key_id="0" * 16, signature="0" * 128)])).encode()


def payload(revision, skills):
    return dict(schema_version=1, checks_version=1, environment="staging", revision=revision,
                issued_at="2026-11-16T00:00:00Z", expires_at="2026-11-30T00:00:00Z",
                publishers=[], skills=[dict(id=k, sha256=v) for k, v in skills.items()])


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.temp.name) / "payload.json"

    def tearDown(self):
        self.temp.cleanup()

    def validator(self, path, environment):
        item = json.loads(pathlib.Path(path).read_text())
        if item["environment"] != environment:
            raise ValueError("invalid-catalog")
        return item

    def write(self, revision, skills):
        self.path.write_text(json.dumps(payload(revision, skills)))

    def test_dry_run_reports_changes_and_writes_nothing(self):
        storage = FakeStorage(payload(10, {"a": "1", "b": "2"}))
        self.write(11, {"a": "1", "b": "3", "c": "4"})
        report = publish.execute("staging", self.path, storage, self.validator)
        self.assertEqual(storage.writes, [])
        self.assertEqual((report["added"], report["removed"], report["changed"]), (["c"], [], ["b"]))
        self.assertFalse(report["published"])
        self.assertEqual(report["target"], "gs://tc-staging-updates-20261022/skills-catalog/staging.json")

    def test_publish_signs_and_uses_the_generation_precondition(self):
        storage = FakeStorage(payload(10, {}))
        self.write(11, {"a": "1"})
        report = publish.execute("staging", self.path, storage, self.validator, apply=True, signer=fake_signer)
        self.assertTrue(report["published"])
        self.assertEqual(storage.writes[0][2], "7")
        self.assertIn("payload", json.loads(storage.writes[0][1]))

    def test_revision_must_increase_and_initial_is_only_for_the_first(self):
        storage = FakeStorage(payload(10, {}))
        self.write(10, {})
        with self.assertRaisesRegex(ValueError, "increasing-revision"):
            publish.execute("staging", self.path, storage, self.validator)
        self.write(11, {})
        with self.assertRaisesRegex(ValueError, "initial-refused"):
            publish.execute("staging", self.path, storage, self.validator, initial=True)
        with self.assertRaisesRegex(ValueError, "current-catalog-missing"):
            publish.execute("staging", self.path, FakeStorage(), self.validator)
        self.assertFalse(publish.execute("staging", self.path, FakeStorage(), self.validator, initial=True)["published"])

    def test_publish_needs_a_signer_and_the_right_environment(self):
        storage = FakeStorage(payload(10, {}))
        self.write(11, {})
        with self.assertRaisesRegex(ValueError, "signer-required"):
            publish.execute("staging", self.path, storage, self.validator, apply=True)
        with self.assertRaises(ValueError):
            publish.execute("production", self.path, storage, self.validator)
        self.assertEqual(storage.writes, [])

    def test_a_conflicting_write_is_not_reported_published(self):
        storage = FakeStorage(payload(10, {}))
        storage.conflict = True
        self.write(11, {})
        with self.assertRaisesRegex(ValueError, "generation-precondition"):
            publish.execute("staging", self.path, storage, self.validator, apply=True, signer=fake_signer)


if __name__ == "__main__":
    unittest.main()
