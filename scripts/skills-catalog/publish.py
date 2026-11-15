#!/usr/bin/env python3
"""Preview, sign (Cloud KMS) and explicitly publish one environment's Skills catalog.

The payload comes from `trace-commons-skills-catalog export`. A dry run (the
default) reads the current object's metadata and writes nothing. `--publish`
signs through the owner's KMS key, has the CLI check the signature against a
key this build pins, verifies the document exactly as a daemon would, and
uploads with a generation precondition. Runbook: docs/operator/skills-catalog.md.
"""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGETS = {
    "production": "gs://tracecommons-flatpak/skills-catalog/production.json",
    "staging": "gs://tc-staging-updates-20261022/skills-catalog/staging.json",
    "testing": "gs://tc-staging-updates-20261022/skills-catalog/testing.json",
}
MAX_BYTES = 4 * 1024 * 1024


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate-catalog-key")
        result[key] = value
    return result


class GcloudStorage:
    def __init__(self, account=None):
        self.account = account

    def command(self, *args):
        command = ["gcloud", "storage", *args, "--verbosity=error"]
        if self.account:
            command.append("--account=" + self.account)
        return command

    def read(self, target):
        result = subprocess.run(self.command("objects", "describe", target, "--format=json"), capture_output=True, text=True, timeout=30)
        if result.returncode:
            if re.search(r"\b404\b", result.stderr):
                return "0", None
            raise ValueError("could-not-read-current-catalog-metadata")
        metadata = json.loads(result.stdout)
        generation = str(metadata["generation"])
        if not generation.isdigit():
            raise ValueError("invalid-object-generation")
        if int(metadata.get("size", 0)) > MAX_BYTES:
            raise ValueError("current-catalog-too-large")
        raw = subprocess.check_output(self.command("cat", target + "#" + generation), timeout=60)
        if len(raw) > MAX_BYTES:
            raise ValueError("current-catalog-too-large")
        return generation, raw

    def write(self, target, path, generation):
        subprocess.run(self.command("cp", "--content-type=application/json", "--cache-control=public,max-age=300", "--if-generation-match=" + generation, str(path), target), check=True, timeout=120)


def cli_validator(binary):
    """The CLI's own payload check: it refuses anything a client would."""
    def validate(path, environment):
        with tempfile.TemporaryDirectory(prefix="skills-catalog-check-") as directory:
            subprocess.run([str(binary), "skills-catalog", "signing-input", "--file", str(path), "--environment", environment, "--output", str(pathlib.Path(directory) / "input")], check=True, timeout=30, capture_output=True)
        return json.loads(pathlib.Path(path).read_bytes(), object_pairs_hook=unique)
    return validate


def current_payload(raw):
    """The payload inside the current object, read only for its revision and
    the change report. Its signature is not what this run relies on."""
    document = json.loads(raw, object_pairs_hook=unique)
    if not isinstance(document, dict) or not isinstance(document.get("payload"), str):
        raise ValueError("invalid-current-catalog")
    payload = json.loads(document["payload"], object_pairs_hook=unique)
    if not isinstance(payload, dict):
        raise ValueError("invalid-current-catalog")
    return payload


def skill_hashes(payload):
    return {skill["id"]: skill["sha256"] for skill in payload.get("skills", [])}


class KmsSigner:
    """Sign through Cloud KMS. There is no key file: the key never leaves KMS."""
    def __init__(self, cli, location, keyring, key, version, public_key_pem, project=None, account=None):
        self.cli, self.location, self.keyring, self.key, self.version = cli, location, keyring, key, version
        self.public_key_pem, self.project, self.account = public_key_pem, project, account

    def __call__(self, payload, environment):
        with tempfile.TemporaryDirectory(prefix="skills-catalog-sign-") as directory:
            directory = pathlib.Path(directory)
            signing_input, signature, document = directory / "input", directory / "signature", directory / "signed.json"
            subprocess.run([str(self.cli), "skills-catalog", "signing-input", "--file", str(payload), "--environment", environment, "--output", str(signing_input)], check=True, timeout=30)
            command = ["gcloud", "kms", "asymmetric-sign", "--location=" + self.location, "--keyring=" + self.keyring, "--key=" + self.key, "--version=" + str(self.version), "--input-file=" + str(signing_input), "--signature-file=" + str(signature), "--verbosity=error"]
            if self.project:
                command.append("--project=" + self.project)
            if self.account:
                command.append("--account=" + self.account)
            subprocess.run(command, check=True, timeout=60)
            subprocess.run([str(self.cli), "skills-catalog", "assemble", "--file", str(payload), "--environment", environment, "--signature", str(signature), "--public-key-pem", str(self.public_key_pem), "--output", str(document)], check=True, timeout=30)
            # Exactly what a daemon of this build checks.
            subprocess.run([str(self.cli), "skills-catalog", "verify", "--file", str(document), "--environment", environment], check=True, timeout=30, capture_output=True)
            return document.read_bytes()


def execute(environment, path, storage, validate, apply=False, initial=False, signer=None):
    target = TARGETS[environment]
    with pathlib.Path(path).open("rb") as stream:
        raw = stream.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES:
        raise ValueError("catalog-too-large")
    # Snapshot the reviewed bytes; validation and upload never re-open the input.
    with tempfile.TemporaryDirectory(prefix="skills-catalog-") as directory:
        immutable = pathlib.Path(directory) / "payload.json"
        immutable.write_bytes(raw)
        candidate = validate(immutable, environment)
        if candidate.get("environment") != environment:
            raise ValueError("catalog-environment-mismatch")
        generation, current = storage.read(target)
        previous = None if current is None else current_payload(current)
        if previous is None and not initial:
            raise ValueError("current-catalog-missing: daemons keep their highest accepted revision and refuse a lower one; pass --initial only for the very first publish of this environment")
        if previous is not None and initial:
            raise ValueError("initial-refused: a current catalog exists, so the revision must exceed its revision")
        if previous is not None:
            if previous.get("environment") != environment or type(previous.get("revision")) is not int:
                raise ValueError("invalid-current-catalog")
            if candidate["revision"] <= previous["revision"]:
                raise ValueError("publish-requires-increasing-revision")
        before = skill_hashes(previous or {})
        after = skill_hashes(candidate)
        report = dict(
            environment=environment, target=target, revision=candidate["revision"],
            expires_at=candidate.get("expires_at"), expected_generation=generation,
            sha256=hashlib.sha256(raw).hexdigest(), bytes=len(raw),
            publishers=len(candidate.get("publishers", [])), skills=len(after),
            added=sorted(set(after) - set(before)), removed=sorted(set(before) - set(after)),
            changed=sorted(k for k in set(after) & set(before) if after[k] != before[k]),
            signed=False, published=False,
        )
        print(json.dumps(report, indent=2), flush=True)
        if apply:
            if signer is None:
                raise ValueError("signer-required: daemons refuse an unsigned catalog")
            validate(immutable, environment)
            signed = pathlib.Path(directory) / "signed.json"
            signed.write_bytes(signer(immutable, environment))
            if len(signed.read_bytes()) > MAX_BYTES:
                raise ValueError("signed-catalog-too-large")
            report["signed"] = True
            storage.write(target, signed, generation)
            report["published"] = True
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--environment", choices=TARGETS, required=True)
    parser.add_argument("--payload", type=pathlib.Path, required=True)
    parser.add_argument("--cli", type=pathlib.Path, required=True)
    parser.add_argument("--account", help="Already authenticated gcloud account; does not change defaults")
    parser.add_argument("--publish", action="store_true")
    parser.add_argument("--initial", action="store_true", help="Allow publishing when no object exists; only for the very first publish")
    parser.add_argument("--kms-location")
    parser.add_argument("--kms-keyring")
    parser.add_argument("--kms-key")
    parser.add_argument("--kms-key-version", default="1")
    parser.add_argument("--kms-project")
    parser.add_argument("--public-key-pem", type=pathlib.Path, help="The signing key's PUBLIC KEY PEM; must be pinned for the environment")
    args = parser.parse_args()
    try:
        signer = None
        if args.publish:
            missing = [name for name in ("kms_location", "kms_keyring", "kms_key", "public_key_pem") if getattr(args, name) is None]
            if missing:
                raise ValueError("signing-key-required: --" + ", --".join(name.replace("_", "-") for name in missing))
            signer = KmsSigner(args.cli.resolve(), args.kms_location, args.kms_keyring, args.kms_key, args.kms_key_version, args.public_key_pem, args.kms_project, args.account)
        result = execute(args.environment, args.payload, GcloudStorage(args.account), cli_validator(args.cli.resolve()), args.publish, args.initial, signer)
        if result["published"]:
            print("Published revision", result["revision"], "to", result["environment"])
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
