"""`pipeline.py promote` and `pipeline.py hf-pin record`: the operator-run
production checks (spec 2026-10-08, Slice B, sections 2 and 5).

Qualification is two runs of one code revision (spec section 2, R-1): the
mechanics run (`pipeline.py qualify`, CI or the lab, reference dependencies,
no network) and the production run made here, on the operator host, against
the production package, with network. The production run holds the package
checks (`checks.py`'s `digests_required`) and the three promotion-only checks
(`PROMOTION_ONLY_CHECKS` in `versioned_pipeline_qualification.rs`).

Subcommands, each re-runnable alone into the run `init` created:

- `init`: records the run id, start time, code revision and the package's
  three digests (`promote-run.json`), and keeps a copy of the package.
- `package-checks`: the four package checks re-run with the harness switched
  to the production assembly (`TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY=
  production`, B-2), over the run's package and the components
  `PipelineGateComponents::from_env` builds from the deployment's env file
  (`--env-file`; only `envfile.PACKAGE_CHECK_VARIABLES` is read), with a
  fresh usearch index per check inside the run, and the committed HF network
  pin. Each result must name the run's
  package and its evidence must say `harness_assembly: production`.
- `hf-canary`: `pipeline_hf_network_canary`, a fresh download of the network
  pin, every digest compared.
- `remote-restore`: `pipeline_remote_restore`, from the measurement the
  remote restore harness (`run_remote_restore_harness`) writes: a seed into
  the live store, the database dump and restore, the ciphertext copy into the
  scratch store, and the resume against it.
- `adapters`: collects `pipeline_production_adapters`, which only the deployed
  ingest's boot writes; it never starts ingest.
- `sign`: signs the production run's results with the operator's check key.
- `assemble`: builds the submission from the mechanics run and this run.

Every subcommand refuses to start when `CI` is set (`promote_refused_in_ci`),
so no CI job makes a network call or reaches a bucket. Every failure is a
safe label; store names, paths and URLs never reach a label, a result, or
the terminal.
"""

from __future__ import annotations

import json
import os
import re
import secrets
import shutil
import struct
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, NamedTuple

from . import cargo, checks, envfile, environment
from .corpus import PIN_SCHEMA, export_command, validate_hf_manifest
from .environment import Environment, Run, child_environment, run_child
from .errors import ToolingError, require
from .files import atomic_write, sha256_digest
from .results import SCHEMA as RESULT_SCHEMA
from .results import (
    _is_the_accepted_result,
    _parse_observed_at,
    _read_json_file,
    canonical,
    load_results,
    require_current_pass_results,
    validate_evidence,
)

# The cargo arguments of every ingest test binary `promote` starts: the
# pilot's feature set (spec B-D1), so the signing step and the restore
# harness run on the build the deployment runs.
PROMOTE_FEATURES = ("--features", "near-ai-scorer,gcs-client,gcp-kms")
PROMOTE_CARGO_ARGS = ("-p", "trace-commons-server", "--bin", "trace-commons-ingest", *PROMOTE_FEATURES)

PROMOTE_RUN_SCHEMA = "trace_commons.pipeline_promote_run.v1"
PROMOTE_RUN_FILE = "promote-run.json"
PACKAGE_FILE = "signed-package.json"
TRUSTED_KEY_FILE = "trusted-package-key.json"

ADAPTERS_CHECK_ID = "pipeline_production_adapters"
REMOTE_RESTORE_CHECK_ID = "pipeline_remote_restore"
HF_CANARY_CHECK_ID = "pipeline_hf_network_canary"
# `PROMOTION_ONLY_CHECKS` in the Rust source (a self-test requires agreement).
PROMOTION_ONLY_CHECK_IDS = (ADAPTERS_CHECK_ID, REMOTE_RESTORE_CHECK_ID, HF_CANARY_CHECK_ID)

_PIN_DIR = environment.ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl"
HF_LOCAL_PIN = _PIN_DIR / "pin-local.json"
# The committed network pin, relative to the repository root: the only pin
# `hf-canary` and `package-checks` accept.
HF_NETWORK_PIN_PATH = Path("crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-network.json")
# Every digest a network pin carries, each compared by the canary.
HF_PIN_DIGEST_FIELDS = (
    "source_digest",
    "configuration_digest",
    "order_digest",
    "bootstrap_corpus_digest",
    "holdout_corpus_digest",
)
HF_CANARY_SCHEMA = "trace_commons.pipeline_hf_network_canary.v1"

REMOTE_RESTORE_SCHEMA = "trace_commons.pipeline_remote_restore.v1"
REMOTE_RESTORE_REPORT_SCHEMA = "trace_commons.pipeline_remote_restore_report.v1"
# The object store kinds that are a remote restore. Only GCS is compiled
# (`gcs-client`); a file-system store is a local copy, which is what the
# local restore drill already proves.
REMOTE_STORE_KINDS = frozenset({"gcs"})
# The harness's three ignored ingest tests: the local restore drill's seed
# and resume, run with a remote store in place of the artifact root, and the
# copy between them.
RESTORE_SEED = "tests::pipeline_restore_pg_tests::pipeline_restore_seed"
RESTORE_RESUME = "tests::pipeline_restore_pg_tests::pipeline_restore_resume"
REMOTE_RESTORE_RUN = "tests::pipeline_restore_pg_tests::pipeline_remote_restore_run"
RESTORE_FINGERPRINT_SCHEMA = "trace_commons.pipeline_restore_fingerprint.v1"
REMOTE_RESTORE_COPY_SCHEMA = "trace_commons.pipeline_remote_restore_copy.v1"
REMOTE_RESTORE_RESUME_SCHEMA = "trace_commons.pipeline_remote_restore_resume.v1"
# The only ambient variables the harness passes on: which key wrapper the
# deployment uses, and the KMS key it names (a resource name, never key
# material). Credentials stay out (`child_environment`); on the operator
# host they come from Application Default Credentials.
KEK_SELECTION_VARIABLES = ("TRACE_COMMONS_KEK_PROVIDER", "TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME")

# The harness variables `package-checks` sets (`versioned_pipeline_harness.rs`).
HARNESS_ASSEMBLY_VAR = "TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY"
HARNESS_PACKAGE_PATH_VAR = "TRACE_COMMONS_PIPELINE_HARNESS_PACKAGE_PATH"
HARNESS_TRUSTED_KEY_PATH_VAR = "TRACE_COMMONS_PIPELINE_HARNESS_TRUSTED_KEY_PATH"
# Where `PipelineGateComponents::from_env` opens the pipeline index: always a
# directory inside the run, never the deployment's.
PIPELINE_INDEX_ROOT_VAR = "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT"
# The env file variables without which the real scorer cannot start
# (`NearAiGateSharedComponents::from_env`); the rest have defaults there.
PACKAGE_CHECK_REQUIRED_VARIABLES = (
    "TRACE_COMMONS_NEAR_AI_BASE_URL",
    "TRACE_COMMONS_NEAR_AI_API_KEY",
    "TRACE_COMMONS_NEAR_AI_MODEL",
)
# Where the run keeps the production harness's usearch indexes, one
# directory per check, emptied at the start of every `package-checks`.
INDEX_DIR = "indexes"

_RUN_ID = re.compile(r"q[0-9a-f]{8}\Z")
_HASH = re.compile(r"sha256:[0-9a-f]{64}\Z")
_REVISION = re.compile(r"[0-9a-f]{40}\Z")
_KIND = re.compile(r"[a-z][a-z0-9_]{0,31}\Z")
# A bucket name, then optional `/`-separated prefix segments: never a URL
# (`_reject_url_like_arguments` refuses `scheme://` first), never `..`.
_BUCKET = re.compile(r"[a-z0-9][a-z0-9._-]{1,61}[a-z0-9]\Z")
_PREFIX_SEGMENT = re.compile(r"[A-Za-z0-9_-][A-Za-z0-9._-]{0,127}\Z")
_PHASES = ("admission", "review", "score", "settle")


class Hooks(NamedTuple):
    """What `pipeline.py` lends the subcommands: its signing-flag parser
    (`signing_options`), its signing step (`sign(run, accepted,
    signing, cargo_args)`, which signs, publishes, and removes what it signed
    on failure; returns the count), and the runner of the four package checks
    (`package_checks(run, *, harness_env, cargo_features, network_pin,
    postgres_admin_url)`, the runners `qualify` uses with the production
    harness variables added)."""

    signing_options: Callable
    sign: Callable
    package_checks: Callable


# ---------------------------------------------------------------------------
# The check lists.
# ---------------------------------------------------------------------------


def package_check_ids():
    """The required checks that test the candidate package (`checks.py`'s
    `digests_required`), sorted."""
    return tuple(sorted(check_id for check_id, spec in checks.required_specs().items() if spec.digests_required))


def production_check_ids():
    """What the production run holds: the package checks re-run on the
    production assembly, and the three promotion-only checks."""
    return (*package_check_ids(), *PROMOTION_ONLY_CHECK_IDS)


def mechanics_check_ids():
    """What the mechanics run (`qualify`) contributes: every check it runs
    that names no package."""
    package = set(package_check_ids())
    return tuple(sorted(check_id for check_id in checks.REQUIRED_CHECK_IDS if check_id not in package))


# ---------------------------------------------------------------------------
# The package.
# ---------------------------------------------------------------------------


_BUNDLE_MANIFEST_FORMAT_VERSION = 2
_ARTIFACT_HEX = re.compile(r"(?:[0-9a-f]{2})*\Z")


def _encode_len(output, length):
    """`encode_len` in `trace-commons-gate-api/src/pipeline.rs`: every length
    and count is a big-endian `u64`."""
    output += struct.pack(">Q", length)


def _encode_string(output, value):
    """`encode_string`: the UTF-8 byte length, then the bytes. A string with
    no UTF-8 encoding (a lone surrogate, which `json.loads` accepts) is
    refused: a Rust `String` cannot hold one."""
    require(isinstance(value, str), "promote_package_invalid")
    try:
        data = value.encode()
    except UnicodeEncodeError as error:
        raise ToolingError("promote_package_invalid") from error
    _encode_len(output, len(data))
    output += data


def _encode_list(output, values):
    """`encode_list`: sorted, and a duplicate entry is refused, never
    dropped. Python orders `str` by code point, which is the byte order of
    their UTF-8 encodings, so this sorts as Rust's `String` does."""
    require(isinstance(values, list) and all(isinstance(value, str) for value in values), "promote_package_invalid")
    values = sorted(values)
    require(all(a != b for a, b in zip(values, values[1:])), "promote_package_invalid")
    _encode_len(output, len(values))
    for value in values:
        _encode_string(output, value)


def _manifest_canonical_bytes(manifest):
    """`BundleManifest::canonical_bytes`: the domain separator, the format
    version as a big-endian `u32`, the four policies in phase order, then the
    pinned instruments in identifier order, each ending in its `decimals` as
    one raw byte. The descriptor checks that `encode_instrument` also applies
    are left to the server, which verifies the package when it arrives."""
    require(isinstance(manifest, dict), "promote_package_invalid")
    version = manifest.get("format_version")
    require(
        isinstance(version, int) and not isinstance(version, bool) and version == _BUNDLE_MANIFEST_FORMAT_VERSION,
        "promote_package_invalid",
    )
    output = bytearray(b"trace-commons-bundle-manifest\0")
    output += struct.pack(">I", version)
    for phase in _PHASES:
        policy = manifest.get(phase)
        require(isinstance(policy, dict), "promote_package_invalid")
        try:
            policy_id = policy["policy_id"]
            implementation_id = policy["implementation_id"]
            configuration_hash = policy["configuration_hash"]
            data_artifact_hashes = policy["data_artifact_hashes"]
            projection_ids = policy["projection_ids"]
        except KeyError as error:
            raise ToolingError("promote_package_invalid") from error
        require(isinstance(policy_id, str) and policy_id and isinstance(implementation_id, str) and implementation_id, "promote_package_invalid")
        _encode_string(output, policy_id)
        _encode_string(output, implementation_id)
        _encode_string(output, configuration_hash)
        _encode_list(output, data_artifact_hashes)
        _encode_list(output, projection_ids)
    instruments = manifest.get("instruments")
    require(isinstance(instruments, dict), "promote_package_invalid")
    _encode_len(output, len(instruments))
    for instrument_id in sorted(instruments):
        descriptor = instruments[instrument_id]
        require(isinstance(descriptor, dict), "promote_package_invalid")
        try:
            kind, network, contract, decimals = (descriptor[key] for key in ("kind", "network", "contract", "decimals"))
        except KeyError as error:
            raise ToolingError("promote_package_invalid") from error
        require(
            kind in ("nep141", "erc20", "credit_account")
            and isinstance(decimals, int)
            and not isinstance(decimals, bool)
            and 0 <= decimals <= 255,
            "promote_package_invalid",
        )
        _encode_string(output, instrument_id)
        _encode_string(output, kind)
        _encode_string(output, network)
        _encode_string(output, contract)
        output.append(decimals)
    return bytes(output)


def _referenced_artifacts(manifest):
    """`BundleManifest::referenced_artifacts`: every configuration hash and
    data-artifact hash of the four policies."""
    referenced = set()
    for phase in _PHASES:
        policy = manifest[phase]
        for value in (policy["configuration_hash"], *policy["data_artifact_hashes"]):
            require(isinstance(value, str) and _HASH.fullmatch(value) is not None, "promote_package_invalid")
            referenced.add(value)
    return referenced


def bundle_package_hash(package):
    """`BundlePackage::package_hash` in `trace-commons-gate-api/src/pipeline.rs`:
    the SHA-256 of `canonical_bytes`, a domain-separated binary encoding of
    the bundle identifier, the manifest's canonical bytes and the sorted
    artifact hashes. It is not a hash of the package's JSON. Like Rust it
    validates first: the bundle identifier must be the manifest's, the
    artifact set must be exactly the manifest's references, and each
    artifact's bytes must hash to its key. Every refusal is
    `promote_package_invalid`. `fixtures/pipeline-package-digests-vector.json`,
    which a Rust test holds to the Rust rule, pins this."""
    require(isinstance(package, dict), "promote_package_invalid")
    bundle_id = package.get("bundle_id")
    manifest = package.get("manifest")
    artifacts = package.get("artifacts")
    manifest_bytes = _manifest_canonical_bytes(manifest)
    require(isinstance(bundle_id, str) and bundle_id == sha256_digest(manifest_bytes), "promote_package_invalid")
    require(isinstance(artifacts, dict) and set(artifacts) == _referenced_artifacts(manifest), "promote_package_invalid")
    for artifact_hash, artifact_hex in artifacts.items():
        require(isinstance(artifact_hex, str) and _ARTIFACT_HEX.fullmatch(artifact_hex) is not None, "promote_package_invalid")
        require(sha256_digest(bytes.fromhex(artifact_hex)) == artifact_hash, "promote_package_invalid")
    output = bytearray(b"trace-commons-bundle-package\0")
    _encode_string(output, bundle_id)
    _encode_len(output, len(manifest_bytes))
    output += manifest_bytes
    _encode_len(output, len(artifacts))
    for artifact_hash in sorted(artifacts):
        _encode_string(output, artifact_hash)
    return sha256_digest(bytes(output))


def package_digests(signed):
    """`(package_hash, configuration_digest, dependency_digest)` of a signed
    package, by `package_digests` in `versioned_pipeline_qualification.rs`:
    the package hash (`bundle_package_hash`, which must equal the
    signature's `package_hash`), the canonical hash of the four phase
    configuration hashes, and the canonical hash of the sorted Score
    data-artifact hashes. The Ed25519 signature is not verified here (the
    standard library has no Ed25519); the server verifies it against its
    package trust store when the submission arrives."""
    try:
        package = signed["package"]
        signature = signed["signature"]
        manifest = package["manifest"]
        configuration = {phase: manifest[phase]["configuration_hash"] for phase in _PHASES}
        artifacts = sorted(manifest["score"]["data_artifact_hashes"])
        claimed = signature["package_hash"]
    except (KeyError, TypeError) as error:
        raise ToolingError("promote_package_invalid") from error
    require(
        isinstance(claimed, str)
        and _HASH.fullmatch(claimed) is not None
        and all(isinstance(value, str) and _HASH.fullmatch(value) for value in configuration.values())
        and all(isinstance(value, str) and _HASH.fullmatch(value) for value in artifacts),
        "promote_package_invalid",
    )
    require(bundle_package_hash(package) == claimed, "promote_package_hash_mismatch")
    return claimed, sha256_digest(canonical(configuration)), sha256_digest(canonical(artifacts))


# ---------------------------------------------------------------------------
# The production run.
# ---------------------------------------------------------------------------


def refuse_in_ci():
    require("CI" not in os.environ, "promote_refused_in_ci")


def runs_dir():
    return environment.ROOT / ".local" / "pipeline" / "runs"


class ProductionRun(NamedTuple):
    run: Run
    package: tuple


def open_run(run_id):
    """The run `init` created, reopened: its id, start time and package from
    `promote-run.json`, and the code revision, which must still be the
    working tree's (`promote_code_revision_changed`)."""
    require(isinstance(run_id, str) and _RUN_ID.fullmatch(run_id) is not None, "promote_run_id_invalid")
    run_dir = runs_dir() / run_id
    record_path = run_dir / PROMOTE_RUN_FILE
    require(record_path.is_file(), "promote_run_missing")
    record = _read_json_file(record_path, "promote_run_invalid")
    keys = {"schema", "run_id", "started_at", "code_revision_hash", "package_hash", "configuration_digest", "dependency_digest"}
    require(
        isinstance(record, dict)
        and set(record) == keys
        and record["schema"] == PROMOTE_RUN_SCHEMA
        and record["run_id"] == run_id
        and all(isinstance(record[key], str) and _HASH.fullmatch(record[key]) for key in keys - {"schema", "run_id", "started_at"}),
        "promote_run_invalid",
    )
    try:
        started_at = _parse_observed_at(record["started_at"])
    except ToolingError as error:
        raise ToolingError("promote_run_invalid") from error
    require(environment._code_revision_hash() == record["code_revision_hash"], "promote_code_revision_changed")
    run = Run(run_id=run_id, started_at=started_at, run_dir=run_dir, code_revision_hash=record["code_revision_hash"])
    package = (record["package_hash"], record["configuration_digest"], record["dependency_digest"])
    return ProductionRun(run, package)


def _iso_now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def write_result(production, check_id, evidence, safe_blockers):
    """Writes `<check_id>.evidence.json` and `<check_id>.result.json` into the
    production run's results, as `PipelineCheckEmitter` writes them: status
    `pass` exactly when `safe_blockers` is empty, `fail` otherwise, naming the
    run's package. A re-run replaces the check's previous files, and removes
    every attestation of the run (they signed a set that no longer is)."""
    validate_evidence(evidence)
    require(all(re.fullmatch(r"[a-z0-9_]{1,64}", label) for label in safe_blockers), "safe_blocker_invalid")
    run = production.run
    results_dir = run.results_dir
    results_dir.mkdir(parents=True, exist_ok=True)
    for path in results_dir.glob("*.attestation.json"):
        path.unlink()
    for suffix in ("result", "evidence"):
        (results_dir / f"{check_id}.{suffix}.json").unlink(missing_ok=True)
    package_hash, configuration_digest, dependency_digest = production.package
    raw = {
        "schema": RESULT_SCHEMA,
        "run_id": run.run_id,
        "check_id": check_id,
        "status": "fail" if safe_blockers else "pass",
        "code_revision_hash": run.code_revision_hash,
        "package_hash": package_hash,
        "configuration_digest": configuration_digest,
        "dependency_digest": dependency_digest,
        "observed_at": _iso_now(),
        "evidence_hash": sha256_digest(canonical(evidence)),
        "safe_blockers": list(safe_blockers),
    }
    atomic_write(results_dir / f"{check_id}.evidence.json", canonical(evidence))
    atomic_write(results_dir / f"{check_id}.result.json", canonical(raw))
    print(f"PipelinePromoteCheckOK: check={check_id} status={raw['status']}" if not safe_blockers else
          f"PipelinePromoteCheck: check={check_id} status=fail blockers={','.join(safe_blockers)}")
    require(not safe_blockers, f"check_result_failed:{check_id}")
    return raw


# ---------------------------------------------------------------------------
# Subcommands.
# ---------------------------------------------------------------------------


def init(args, run):
    """Starts a production run: `run` (the one `main` created) becomes it."""
    refuse_in_ci()
    package_path = Path(args.package).resolve()
    key_path = Path(args.trusted_key).resolve()
    signed = _read_json_file(package_path, "promote_package_invalid")
    package_hash, configuration_digest, dependency_digest = package_digests(signed)
    trusted = _read_json_file(key_path, "promote_trusted_key_invalid")
    require(
        isinstance(trusted, dict) and set(trusted) == {"key_id", "public_key_base64url"},
        "promote_trusted_key_invalid",
    )
    atomic_write(run.run_dir / PACKAGE_FILE, package_path.read_bytes())
    atomic_write(run.run_dir / TRUSTED_KEY_FILE, key_path.read_bytes())
    record = {
        "schema": PROMOTE_RUN_SCHEMA,
        "run_id": run.run_id,
        "started_at": run.started_at.astimezone(timezone.utc).isoformat().replace("+00:00", "Z"),
        "code_revision_hash": run.code_revision_hash,
        "package_hash": package_hash,
        "configuration_digest": configuration_digest,
        "dependency_digest": dependency_digest,
    }
    atomic_write(run.run_dir / PROMOTE_RUN_FILE, json.dumps(record, indent=2).encode())
    print(
        f"PipelinePromoteInit: run_id={run.run_id} code_revision_hash={run.code_revision_hash} "
        f"package_hash={package_hash}"
    )
    print(
        "PipelinePromoteAdapters: boot the deployed ingest once with "
        f"TRACE_COMMONS_PIPELINE_CHECK_RUN_ID={run.run_id} "
        f"TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH={run.code_revision_hash} "
        "and TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR set, then copy its two "
        f"{ADAPTERS_CHECK_ID} files into this run's results directory"
    )


def require_production_assembly(production, loaded):
    """Each package check's result in `loaded` came from the production
    assembly: its evidence says `harness_assembly: production`
    (`promote_harness_assembly_not_production:<id>` otherwise). The package
    triple alone does not show it: a reference-assembly harness given the
    production package would name the same triple over reference doubles."""
    for check_id in package_check_ids():
        require(check_id in loaded, f"check_result_missing:{check_id}")
        evidence = _read_json_file(
            production.run.results_dir / f"{check_id}.evidence.json", "check_evidence_malformed"
        )
        require(
            isinstance(evidence, dict) and evidence.get("harness_assembly") == "production",
            f"promote_harness_assembly_not_production:{check_id}",
        )


def committed_network_pin(given):
    """The committed `pin-network.json`, which the run's code revision
    covers, validated. `given` (`--pin`), when set, must hold its exact bytes
    (`hf_network_pin_not_committed`): a pin `hf-pin record` wrote for another
    revision is internally consistent, and the HF corpus check would then
    test a corpus nobody reviewed."""
    pin_path = environment.ROOT / HF_NETWORK_PIN_PATH
    _load_network_pin(pin_path)
    if given is not None:
        given = Path(given).resolve()
        require(given.is_file(), "hf_network_pin_not_committed")
        require(sha256_digest(given.read_bytes()) == sha256_digest(pin_path.read_bytes()), "hf_network_pin_not_committed")
    return pin_path


def make_package_checks(hooks):
    def package_checks(args, run):
        """The four package checks on the production assembly (spec B-D1).
        Refused before anything starts when the network pin is missing or
        the env file lacks the NEAR AI endpoint or key; the env file's other
        variables never reach a child. A re-run replaces the four results
        and removes every attestation of the run."""
        refuse_in_ci()
        production = open_run(args.run_id)
        pin_path = committed_network_pin(args.pin)
        variables = envfile.allowlisted(envfile.read_env_file(args.env_file), envfile.PACKAGE_CHECK_VARIABLES)
        require(
            all(variables.get(name, "").strip() for name in PACKAGE_CHECK_REQUIRED_VARIABLES),
            "promote_env_file_incomplete",
        )
        run_dir = production.run.run_dir
        index_root = run_dir / INDEX_DIR
        shutil.rmtree(index_root, ignore_errors=True)
        results_dir = production.run.results_dir
        results_dir.mkdir(parents=True, exist_ok=True)
        for path in results_dir.glob("*.attestation.json"):
            path.unlink()
        for check_id in package_check_ids():
            for suffix in ("result", "evidence"):
                (results_dir / f"{check_id}.{suffix}.json").unlink(missing_ok=True)
        harness_env = {
            **variables,
            HARNESS_ASSEMBLY_VAR: "production",
            HARNESS_PACKAGE_PATH_VAR: str(run_dir / PACKAGE_FILE),
            HARNESS_TRUSTED_KEY_PATH_VAR: str(run_dir / TRUSTED_KEY_FILE),
            PIPELINE_INDEX_ROOT_VAR: str(index_root),
        }
        hooks.package_checks(
            production.run,
            harness_env=harness_env,
            cargo_features=PROMOTE_FEATURES,
            network_pin=pin_path,
            postgres_admin_url=args.postgres_admin_url,
        )
        loaded = load_results(production.run)
        wanted = package_check_ids()
        require_current_pass_results(
            production.run, loaded, {check_id: checks.CheckSpec(check_id, True) for check_id in wanted}
        )
        for check_id in wanted:
            result = loaded[check_id]
            require(
                (result.package_hash, result.configuration_digest, result.dependency_digest) == production.package,
                "promote_package_mismatch",
            )
        require_production_assembly(production, loaded)
        production.run.require_code_revision_unchanged()
        print(f"PipelinePromotePackageChecksOK: checks={','.join(wanted)} package_hash={production.package[0]}")

    return package_checks


def run_hf_export(run, step, fields, output_dir, cache_dir):
    """Runs the export binary for the pin fields `fields` with no expected
    digests, downloading into `cache_dir`. The self-tests replace this."""
    Path(cache_dir).mkdir(parents=True, exist_ok=True)
    command = export_command(fields, output_dir, cache_dir, expected_digests=False)
    run_child(run, step, command, child_environment({}))


def _pin_source_fields(pin):
    return {key: pin[key] for key in (
        "repository", "revision", "split", "translator", "bootstrap_count", "holdout_count",
        "min_words", "max_words", "expected_instrument_count",
    )}


def _download(run, step, fields, work_dir):
    """A fresh download of `fields` into `work_dir` (its cache and output
    replaced): the manifest the export wrote, checked, and the number of
    JSONL files the download left in the cache."""
    shutil.rmtree(work_dir, ignore_errors=True)
    cache_dir = work_dir / "hf-cache"
    output_dir = work_dir / "export"
    run_hf_export(run, step, fields, output_dir, cache_dir)
    manifest = _read_json_file(output_dir / "source-manifest.json", "hf_manifest_invalid")
    validate_hf_manifest(manifest)
    require(
        {key: manifest["source"][key] for key in fields} == fields,
        "hf_manifest_source_mismatch",
    )
    downloaded = sum(1 for path in cache_dir.rglob("*.jsonl") if path.is_file()) if cache_dir.is_dir() else 0
    return manifest, downloaded, cache_dir


def hf_pin_record(args, run):
    """Downloads the pinned revision once and writes `pin-network.json`:
    `pin-local.json`'s fields less `local_jsonl_dir`, the revision given, and
    the five digests the download computed. Never overwrites a file. The owner
    commits the pin in a PR; a canary never regenerates it."""
    refuse_in_ci()
    require(_REVISION.fullmatch(args.revision or "") is not None, "hf_pin_revision_invalid")
    output = Path(args.output).resolve()
    require(not os.path.lexists(output), "hf_pin_output_exists")
    local = _read_json_file(HF_LOCAL_PIN, "hf_local_pin_invalid")
    require(isinstance(local, dict) and local.get("schema") == PIN_SCHEMA, "hf_local_pin_invalid")
    fields = _pin_source_fields(local)
    fields["revision"] = args.revision
    manifest, _, _ = _download(run, "hf_pin_record", fields, run.run_dir / "hf-pin-record")
    pin = {"schema": PIN_SCHEMA, **fields}
    for field in HF_PIN_DIGEST_FIELDS:
        pin[field] = manifest[field]
    require(not os.path.lexists(output), "hf_pin_output_exists")
    atomic_write(output, (json.dumps(pin, indent=2) + "\n").encode())
    print(f"PipelineHfPinRecorded: revision={args.revision} source_digest={pin['source_digest']}")


def _load_network_pin(path):
    pin = _read_json_file(path, "hf_network_pin_missing") if Path(path).is_file() else None
    require(pin is not None, "hf_network_pin_missing")
    require(isinstance(pin, dict) and "local_jsonl_dir" not in pin, "hf_network_pin_has_local_dir")
    expected = {
        "schema", "repository", "revision", "split", "translator", "bootstrap_count", "holdout_count",
        "min_words", "max_words", "expected_instrument_count", *HF_PIN_DIGEST_FIELDS,
    }
    require(
        set(pin) == expected
        and pin["schema"] == PIN_SCHEMA
        and isinstance(pin["repository"], str)
        and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,95}/[A-Za-z0-9][A-Za-z0-9_.-]{0,95}", pin["repository"])
        and isinstance(pin["revision"], str)
        and _REVISION.fullmatch(pin["revision"])
        and all(isinstance(pin[field], str) and _HASH.fullmatch(pin[field]) for field in HF_PIN_DIGEST_FIELDS),
        "hf_network_pin_invalid",
    )
    return pin


def hf_canary(args, run):
    """`pipeline_hf_network_canary`: downloads the network pin's revision
    again, into a fresh cache inside the production run, and compares every
    digest with the pin. A digest that moved is `fail` with
    `hf_pin_digest_mismatch_<field>`; a download that left no JSONL file in
    the cache is `fail` with `hf_network_download_missing`.

    The pin is the committed `pin-network.json`, which the run's code
    revision covers. A `--pin` whose bytes differ from it is refused
    (`hf_network_pin_not_committed`) before anything is downloaded: a pin
    `hf-pin record` wrote for another revision matches its own download in
    every digest, and a canary would certify a pin nobody reviewed."""
    refuse_in_ci()
    production = open_run(args.run_id)
    pin_path = environment.ROOT / HF_NETWORK_PIN_PATH
    pin = _load_network_pin(pin_path)
    if args.pin is not None:
        given = Path(args.pin).resolve()
        require(given.is_file(), "hf_network_pin_not_committed")
        require(sha256_digest(given.read_bytes()) == sha256_digest(pin_path.read_bytes()), "hf_network_pin_not_committed")
    work_dir = production.run.run_dir / "hf-canary"
    manifest, downloaded, cache_dir = _download(production.run, "hf_network_canary", _pin_source_fields(pin), work_dir)
    blockers = [f"hf_pin_digest_mismatch_{field}" for field in HF_PIN_DIGEST_FIELDS if manifest[field] != pin[field]]
    if downloaded == 0:
        blockers.append("hf_network_download_missing")
    owner, name = pin["repository"].split("/", 1)
    evidence = {
        "schema": HF_CANARY_SCHEMA,
        # The public dataset coordinates, split at the `/` the evidence
        # alphabet does not allow.
        "repository_owner": owner,
        "repository_name": name,
        "revision": pin["revision"],
        "pin_hash": sha256_digest(pin_path.read_bytes()),
        "source_digest": manifest["source_digest"],
        "order_digest": manifest["order_digest"],
        "bootstrap_corpus_digest": manifest["bootstrap_corpus_digest"],
        "holdout_corpus_digest": manifest["holdout_corpus_digest"],
        "downloaded_file_count": downloaded,
        "cache_dir_inside_run": cache_dir.resolve().is_relative_to(production.run.run_dir.resolve()),
    }
    if not evidence["cache_dir_inside_run"]:
        blockers.append("hf_network_cache_outside_run")
    write_result(production, HF_CANARY_CHECK_ID, evidence, blockers)


def _store_name(value):
    """A bucket name with optional prefix segments, as given; anything else
    is `remote_restore_store_name_invalid`."""
    require(isinstance(value, str) and value, "remote_restore_store_name_invalid")
    bucket, *prefix = value.split("/")
    require(_BUCKET.fullmatch(bucket) is not None, "remote_restore_store_name_invalid")
    require(all(_PREFIX_SEGMENT.fullmatch(segment) and segment not in (".", "..") for segment in prefix),
            "remote_restore_store_name_invalid")
    return value


def _overlaps(first, second):
    a, b = first.split("/"), second.split("/")
    shorter = min(len(a), len(b))
    return a[:shorter] == b[:shorter]


def _read_seed_fingerprint(path):
    """The two hashes the remote restore needs from the seed's fingerprint
    file (the local drill reads the rest)."""
    value = _read_json_file(path, "restore_fingerprint_invalid")
    require(
        isinstance(value, dict)
        and value.get("schema") == RESTORE_FINGERPRINT_SCHEMA
        and all(
            isinstance(value.get(key), str) and _HASH.fullmatch(value[key]) is not None
            for key in ("database_fingerprint", "artifact_fingerprint")
        ),
        "restore_fingerprint_invalid",
    )
    return value


def _read_copy_report(path):
    """`pipeline_remote_restore_run`'s measurement, exactly its fields."""
    value = _read_json_file(path, "remote_restore_copy_report_invalid")
    hashes = ("artifact_fingerprint", "restored_artifact_fingerprint")
    counts = ("object_count", "kek_unwrap_verified_count")
    require(
        isinstance(value, dict)
        and set(value) == {"schema", "object_store_kind", "versioning_enabled", *hashes, *counts}
        and value["schema"] == REMOTE_RESTORE_COPY_SCHEMA
        and isinstance(value["object_store_kind"], str)
        and _KIND.fullmatch(value["object_store_kind"]) is not None
        and type(value["versioning_enabled"]) is bool
        and all(isinstance(value[key], str) and _HASH.fullmatch(value[key]) for key in hashes)
        and all(type(value[key]) is int and value[key] >= 0 for key in counts),
        "remote_restore_copy_report_invalid",
    )
    return value


def _read_resume_report(path, seed):
    """The remote resume's measurement, exactly its fields. The resume
    already refused a restored store whose fingerprint is not the seed's;
    one that reports another is refused here too."""
    value = _read_json_file(path, "remote_restore_resume_report_invalid")
    hashes = ("database_fingerprint", "artifact_fingerprint")
    counts = ("pending_runs_resumed", "duplicate_effects")
    require(
        isinstance(value, dict)
        and set(value) == {"schema", *hashes, *counts}
        and value["schema"] == REMOTE_RESTORE_RESUME_SCHEMA
        and all(isinstance(value[key], str) and _HASH.fullmatch(value[key]) for key in hashes)
        and all(type(value[key]) is int and value[key] >= 0 for key in counts)
        and value["artifact_fingerprint"] == seed["artifact_fingerprint"],
        "remote_restore_resume_report_invalid",
    )
    return value


def run_remote_restore_harness(
    run, step, source_store, scratch_store, report_path, *, postgres_admin_url=None, double_root=None
):
    """Restores the live store's objects into the scratch store and writes
    the measurement (`REMOTE_RESTORE_REPORT_SCHEMA`) to `report_path`. The
    local restore drill's order (`pipeline.py`'s `run_restore_drill`), in a
    PostgreSQL of its own (a container, or the loopback server
    `postgres_admin_url` names) and with a remote store in place of the
    artifact root:

    1. the seed, on the drill's `_pilot` database, writing its artifacts
       through the deployment's store (the GCS client and the selected key
       wrapper) into the live store, under a namespace fresh for this
       invocation (`pipeline-remote-restore-<run>-<random>`), never beside
       the live objects;
    2. the dump of that database and its restore into `_restored`;
    3. `pipeline_remote_restore_run`: the namespace's objects copied as
       ciphertext into the same namespace of the scratch store, with the
       fingerprint of each side, the unwrap count and both buckets'
       versioning;
    4. the resume against the restored database and the scratch store.

    The seed and the resume serve the reference compatibility candidate
    (no `HARNESS_ASSEMBLY_VAR` reaches them), not the production assembly
    `package-checks` uses: this check's claim is the remote store and the
    key wrapper across a restore, and the production assembly's own restore
    is `package-checks`' `pipeline_restore_drill`. That keeps the NEAR AI
    credentials and the vector index out of this drill.

    Each child is built with `PROMOTE_CARGO_ARGS`. Only the master key, the
    store names, the namespace and the key-wrapper selection cross into the
    children; store names never reach a label or the report. The drill's
    objects stay in both stores (nothing here deletes from a bucket).

    `double_root` (no command line flag reaches it) runs the same three
    processes against the test-only directory double instead of GCS, for a
    local run of the whole harness; its report names the double's kind, which
    `remote_restore` never passes."""
    report_path = Path(report_path)
    work_dir = report_path.parent
    fingerprint_path = work_dir / "seed-fingerprint.json"
    copy_path = work_dir / "copy-report.json"
    resume_path = work_dir / "resume-report.json"
    dump_path = work_dir / f"{run.run_id}.dump"
    shared = {
        # One random key for the three processes: the resume and the copy
        # unwrap what the seed wrapped. Only the children see it.
        "TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX": secrets.token_hex(32),
        "TRACE_COMMONS_PIPELINE_REMOTE_STORE_KIND": "gcs" if double_root is None else "gcs_directory_double",
        "TRACE_COMMONS_PIPELINE_REMOTE_NAMESPACE": f"pipeline-remote-restore-{run.run_id}-{secrets.token_hex(4)}",
        **{key: os.environ[key] for key in KEK_SELECTION_VARIABLES if key in os.environ},
    }
    if double_root is not None:
        shared["TRACE_COMMONS_PIPELINE_REMOTE_DOUBLE_ROOT"] = str(double_root)
    try:
        with Environment(run, postgres_admin_url=postgres_admin_url) as environment_:
            scenario = environment_.scenario(step)
            seed_env = {
                **shared,
                "TRACE_COMMONS_PG_TEST_DATABASE_URL": scenario.runtime_url,
                "TRACE_COMMONS_PIPELINE_REMOTE_ARTIFACT_STORE": source_store,
                "TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH": str(fingerprint_path),
            }
            cargo.cargo_test(
                run, f"{step}_seed", PROMOTE_CARGO_ARGS, RESTORE_SEED, child_environment(seed_env),
                exact=True, ignored=True,
            )
            require(
                scenario.committed_transactions(scenario.pilot_database) >= 5,
                f"database_check_executed_nothing:{step}_seed",
            )
            seed = _read_seed_fingerprint(fingerprint_path)
            try:
                environment_.dump(scenario.pilot_database, dump_path)
                environment_.create_database(scenario.restored_database)
                environment_.restore(dump_path, scenario.restored_database)
            finally:
                dump_path.unlink(missing_ok=True)

            copy_env = {
                **shared,
                "TRACE_COMMONS_PIPELINE_REMOTE_SOURCE_STORE": source_store,
                "TRACE_COMMONS_PIPELINE_REMOTE_SCRATCH_STORE": scratch_store,
                "TRACE_COMMONS_PIPELINE_REMOTE_COPY_REPORT_PATH": str(copy_path),
            }
            cargo.cargo_test(
                run, f"{step}_copy", PROMOTE_CARGO_ARGS, REMOTE_RESTORE_RUN, child_environment(copy_env),
                exact=True, ignored=True,
            )
            copy = _read_copy_report(copy_path)
            # The source the copy read is the one the seed fingerprinted.
            require(copy["artifact_fingerprint"] == seed["artifact_fingerprint"], "remote_restore_source_changed")

            resume_env = {
                **shared,
                "TRACE_COMMONS_PG_TEST_DATABASE_URL": scenario.restored_url,
                "TRACE_COMMONS_PIPELINE_REMOTE_ARTIFACT_STORE": scratch_store,
                "TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH": str(fingerprint_path),
                "TRACE_COMMONS_PIPELINE_REMOTE_RESUME_REPORT_PATH": str(resume_path),
            }
            cargo.cargo_test(
                run, f"{step}_resume", PROMOTE_CARGO_ARGS, RESTORE_RESUME, child_environment(resume_env),
                exact=True, ignored=True,
            )
            require(
                scenario.committed_transactions(scenario.restored_database) >= 5,
                f"database_check_executed_nothing:{step}_resume",
            )
            resume = _read_resume_report(resume_path, seed)
    finally:
        for path in (fingerprint_path, copy_path, resume_path, dump_path):
            path.unlink(missing_ok=True)
    report = {
        "schema": REMOTE_RESTORE_REPORT_SCHEMA,
        "object_store_kind": copy["object_store_kind"],
        "object_count": copy["object_count"],
        "artifact_fingerprint": copy["artifact_fingerprint"],
        "restored_artifact_fingerprint": copy["restored_artifact_fingerprint"],
        "versioning_enabled": copy["versioning_enabled"],
        "kek_unwrap_verified_count": copy["kek_unwrap_verified_count"],
        "seed_database_fingerprint": seed["database_fingerprint"],
        "resumed_database_fingerprint": resume["database_fingerprint"],
        "pending_runs_resumed": resume["pending_runs_resumed"],
        "duplicate_effects": resume["duplicate_effects"],
    }
    atomic_write(report_path, canonical(report) + b"\n")


_REPORT_HASHES = (
    "artifact_fingerprint",
    "restored_artifact_fingerprint",
    "seed_database_fingerprint",
    "resumed_database_fingerprint",
)
_REPORT_COUNTS = ("object_count", "kek_unwrap_verified_count", "pending_runs_resumed", "duplicate_effects")


def _read_remote_report(path):
    report = _read_json_file(path, "remote_restore_report_invalid")
    keys = {"schema", "object_store_kind", "versioning_enabled", *_REPORT_HASHES, *_REPORT_COUNTS}
    require(
        isinstance(report, dict)
        and set(report) == keys
        and report["schema"] == REMOTE_RESTORE_REPORT_SCHEMA
        and isinstance(report["object_store_kind"], str)
        and _KIND.fullmatch(report["object_store_kind"]) is not None
        and type(report["versioning_enabled"]) is bool
        and all(isinstance(report[key], str) and _HASH.fullmatch(report[key]) for key in _REPORT_HASHES)
        and all(type(report[key]) is int and report[key] >= 0 for key in _REPORT_COUNTS),
        "remote_restore_report_invalid",
    )
    return report


def remote_restore(args, run):
    """`pipeline_remote_restore` (spec B-D2): the harness restores the live
    store's objects for one throwaway tenant into the scratch store (never
    the live prefix) and resumes the restored database against it. Pass
    requires a remote store kind, at least one object, equal ciphertext
    fingerprints before and after, every restored object unwrapped by the
    configured key wrapper, versioning on, and the resume equal to the seed
    (one pending run resumed, no duplicate effect). Store names appear only
    as hashes."""
    refuse_in_ci()
    production = open_run(args.run_id)
    source = _store_name(args.source_store)
    scratch = _store_name(args.scratch_store)
    require(not _overlaps(source, scratch), "remote_restore_scratch_overlaps_live_store")
    work_dir = production.run.run_dir / "remote-restore"
    shutil.rmtree(work_dir, ignore_errors=True)
    work_dir.mkdir(parents=True, mode=0o700)
    report_path = work_dir / "remote-restore-report.json"
    run_remote_restore_harness(
        production.run, "remote_restore", source, scratch, report_path,
        postgres_admin_url=args.postgres_admin_url,
    )
    report = _read_remote_report(report_path)
    blockers = []
    if report["object_store_kind"] not in REMOTE_STORE_KINDS:
        blockers.append("remote_restore_store_not_remote")
    if report["object_count"] == 0:
        blockers.append("remote_restore_empty")
    if report["artifact_fingerprint"] != report["restored_artifact_fingerprint"]:
        blockers.append("remote_restore_artifact_fingerprint_mismatch")
    if report["kek_unwrap_verified_count"] != report["object_count"]:
        blockers.append("remote_restore_unwrap_count_mismatch")
    if not report["versioning_enabled"]:
        blockers.append("remote_restore_versioning_disabled")
    if (
        report["seed_database_fingerprint"] != report["resumed_database_fingerprint"]
        or report["pending_runs_resumed"] != 1
        or report["duplicate_effects"] != 0
    ):
        blockers.append("remote_restore_resume_mismatch")
    evidence = {
        "schema": REMOTE_RESTORE_SCHEMA,
        "object_store_kind": report["object_store_kind"],
        "source_store_name_hash": sha256_digest(source.encode()),
        "scratch_store_name_hash": sha256_digest(scratch.encode()),
        "object_count": report["object_count"],
        "artifact_fingerprint": report["artifact_fingerprint"],
        "restored_artifact_fingerprint": report["restored_artifact_fingerprint"],
        "versioning_enabled": report["versioning_enabled"],
        "kek_unwrap_verified_count": report["kek_unwrap_verified_count"],
        "database_fingerprint": report["seed_database_fingerprint"],
        "pending_runs_resumed": report["pending_runs_resumed"],
        "duplicate_effects": report["duplicate_effects"],
    }
    write_result(production, REMOTE_RESTORE_CHECK_ID, evidence, sorted(set(blockers)))


def adapters(args, run):
    """Collects `pipeline_production_adapters`: the deployed ingest's boot
    wrote it, and the owner copied its two files into this run's results.
    It must be this run's, this revision's, name this package, and pass with
    no blocker."""
    refuse_in_ci()
    production = open_run(args.run_id)
    results_dir = production.run.results_dir
    require((results_dir / f"{ADAPTERS_CHECK_ID}.result.json").is_file(), "promote_adapters_result_missing")
    try:
        loaded = load_results(production.run)
    except ToolingError as error:
        raise ToolingError("promote_adapters_result_mismatch") from error
    result = loaded.get(ADAPTERS_CHECK_ID)
    require(
        result is not None
        and result.run_id == production.run.run_id
        and result.code_revision_hash == production.run.code_revision_hash
        and (result.package_hash, result.configuration_digest, result.dependency_digest) == production.package
        and result.status == "pass"
        and not result.safe_blockers,
        "promote_adapters_result_mismatch",
    )
    print(f"PipelinePromoteCheckOK: check={ADAPTERS_CHECK_ID} status=pass")


def _accepted_production_results(production):
    """The production run's results, exactly the production checks, each a
    current pass of this run and revision naming this run's package."""
    run = production.run
    loaded = load_results(run)
    wanted = production_check_ids()
    require_current_pass_results(run, loaded, {check_id: checks.CheckSpec(check_id, True) for check_id in wanted})
    require(set(loaded) == set(wanted), "promote_results_unexpected")
    for check_id in wanted:
        result = loaded[check_id]
        require(
            (result.package_hash, result.configuration_digest, result.dependency_digest) == production.package,
            "promote_package_mismatch",
        )
    require_production_assembly(production, loaded)
    return {check_id: loaded[check_id] for check_id in wanted}


def make_sign(hooks):
    def sign(args, run):
        """Signs the production run's results with the operator's check key
        (the signing step `qualify --signing-key` uses, on the pilot's
        feature set)."""
        refuse_in_ci()
        signing = hooks.signing_options(args)
        require(signing is not None, "signing_key_incomplete")
        production = open_run(args.run_id)
        accepted = _accepted_production_results(production)
        production.run.require_code_revision_unchanged()
        attested = hooks.sign(production.run, accepted, signing, PROMOTE_CARGO_ARGS)
        print(f"PipelinePromoteSignOK: attested={attested}")

    return sign


def _attestation(run, check_id, results_by_id, label_missing):
    path = run.results_dir / f"{check_id}.attestation.json"
    require(path.is_file() and check_id in results_by_id, label_missing)
    raw = _read_json_file(path, "promote_assemble_attestation_invalid")
    require(
        isinstance(raw, dict) and _is_the_accepted_result(raw.get("result"), results_by_id[check_id]),
        "promote_assemble_attestation_invalid",
    )
    return path


def assemble(args, run):
    """The submission's attestations: the mechanics checks from the
    mechanics run, the production checks from this run, 22 files, one per
    required id, written into a new directory with the signed package. A
    missing attestation is `promote_assemble_check_missing:<id>`; a mechanics
    check this run also holds is `promote_assemble_check_doubled:<id>`; two
    code revisions are `promote_assemble_mixed_revision`."""
    refuse_in_ci()
    production = open_run(args.run_id)
    output = Path(args.output).resolve()
    require(not os.path.lexists(output), "promote_assemble_output_exists")
    require(isinstance(args.mechanics_run_id, str) and _RUN_ID.fullmatch(args.mechanics_run_id), "promote_run_id_invalid")
    require(args.mechanics_run_id != production.run.run_id, "promote_assemble_runs_must_differ")
    mechanics_dir = runs_dir() / args.mechanics_run_id
    require((mechanics_dir / "results").is_dir(), "promote_run_missing")
    mechanics = Run(args.mechanics_run_id, production.run.started_at, mechanics_dir, production.run.code_revision_hash)

    production_results = load_results(production.run)
    mechanics_results = load_results(mechanics)
    for check_id in mechanics_check_ids():
        require(check_id not in production_results, f"promote_assemble_check_doubled:{check_id}")
    sources = {}
    for check_id in mechanics_check_ids():
        sources[check_id] = _attestation(mechanics, check_id, mechanics_results, f"promote_assemble_check_missing:{check_id}")
        result = mechanics_results[check_id]
        require(
            result.run_id == mechanics.run_id
            and (result.package_hash, result.configuration_digest, result.dependency_digest) == (None, None, None),
            "promote_assemble_mechanics_result_invalid",
        )
    for check_id in production_check_ids():
        sources[check_id] = _attestation(production.run, check_id, production_results, f"promote_assemble_check_missing:{check_id}")
        result = production_results[check_id]
        require(
            result.run_id == production.run.run_id
            and (result.package_hash, result.configuration_digest, result.dependency_digest) == production.package,
            "promote_package_mismatch",
        )
    revisions = {mechanics_results[check_id].code_revision_hash for check_id in mechanics_check_ids()}
    revisions |= {production_results[check_id].code_revision_hash for check_id in production_check_ids()}
    require(len(revisions) == 1, "promote_assemble_mixed_revision")
    require(len(sources) == len(mechanics_check_ids()) + len(production_check_ids()), "promote_assemble_count_mismatch")

    # A fresh staging directory beside the output (so the final rename stays
    # on one file system), never a fixed name: a directory that already has
    # a name like it is someone else's and is left alone.
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent))
    try:
        for check_id, path in sorted(sources.items()):
            shutil.copyfile(path, staging / path.name)
        shutil.copyfile(production.run.run_dir / PACKAGE_FILE, staging / PACKAGE_FILE)
        os.replace(staging, output)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    print(f"PipelinePromoteAssembleOK: attestations={len(sources)} package_hash={production.package[0]}")


# ---------------------------------------------------------------------------
# The parsers.
# ---------------------------------------------------------------------------

# `main`'s run directory: `promote init` and `hf-pin record` keep it (it is
# the production run, or holds the download), unless they were refused
# before they ran anything; every other `promote` subcommand works in the run
# `init` made and leaves `main`'s unused one behind.
KEEP_UNLESS_REFUSED = "keep_unless_refused"
NEVER_USED = "never_used"


def add_parsers(subparsers, hooks):
    promote_parser = subparsers.add_parser(
        "promote", help="Operator-only production checks for a production package (refused in CI)"
    )
    commands = promote_parser.add_subparsers(dest="promote_command", required=True)

    init_parser = commands.add_parser("init", help="Start a production run for a signed production package")
    init_parser.add_argument("--package", required=True, help="The signed production package.")
    init_parser.add_argument("--trusted-key", dest="trusted_key", required=True, help="The package's trusted key.")
    init_parser.set_defaults(handler=init, run_dir_mode=KEEP_UNLESS_REFUSED)

    def run_parser(name, handler, help_text):
        parser = commands.add_parser(name, help=help_text)
        parser.add_argument("--run-id", dest="run_id", required=True, help="The run `promote init` printed.")
        parser.set_defaults(handler=handler, run_dir_mode=NEVER_USED)
        return parser

    package = run_parser(
        "package-checks", make_package_checks(hooks), "The four package checks on the production assembly"
    )
    package.add_argument(
        "--env-file",
        dest="env_file",
        required=True,
        help="The deployment's env file; only the variables PipelineGateComponents::from_env reads are passed on.",
    )
    package.add_argument(
        "--pin", default=None, help="The network pin; refused unless its bytes are the committed pin-network.json's."
    )
    package.add_argument(
        "--postgres-admin-url",
        dest="postgres_admin_url",
        default=None,
        help="Use this existing PostgreSQL server instead of starting a container.",
    )
    canary = run_parser("hf-canary", hf_canary, "pipeline_hf_network_canary: download the network pin and compare it")
    canary.add_argument(
        "--pin", default=None, help="The network pin; refused unless its bytes are the committed pin-network.json's."
    )
    restore = run_parser("remote-restore", remote_restore, "pipeline_remote_restore: the remote-store restore drill")
    restore.add_argument("--source-store", dest="source_store", required=True, help="The live bucket[/prefix] name.")
    restore.add_argument(
        "--scratch-store", dest="scratch_store", required=True, help="A scratch bucket[/prefix] name, never the live one."
    )
    restore.add_argument(
        "--postgres-admin-url", dest="postgres_admin_url", default=None,
        help="A throwaway loopback PostgreSQL for the drill's own databases (default: a container).",
    )
    run_parser("adapters", adapters, "Collect pipeline_production_adapters from the deployed boot")
    sign = run_parser("sign", make_sign(hooks), "Sign the production run's results")
    sign.add_argument("--signing-key", dest="signing_key", default=None, help="Ed25519 PKCS#8 DER check key.")
    sign.add_argument("--signing-key-id", dest="signing_key_id", default=None, help="The check key's id.")
    sign.add_argument(
        "--evidence-max-age-seconds", dest="evidence_max_age_seconds", type=int, default=24 * 60 * 60,
        help="How long a signed result stays current (default 86400).",
    )
    assemble_parser = run_parser("assemble", assemble, "Build the 22-attestation submission")
    assemble_parser.add_argument(
        "--mechanics-run-id", dest="mechanics_run_id", required=True, help="The signed `qualify` run."
    )
    assemble_parser.add_argument("--output", required=True, help="A new directory for the submission.")

    pin_parser = subparsers.add_parser("hf-pin", help="Record the HF network pin (operator-only, refused in CI)")
    pin_commands = pin_parser.add_subparsers(dest="hf_pin_command", required=True)
    record = pin_commands.add_parser("record", help="Download a revision once and write its pin")
    record.add_argument("--revision", required=True, help="The dataset commit (40 hex characters).")
    record.add_argument("--output", required=True, help="Where to write the pin (never overwritten).")
    record.set_defaults(handler=hf_pin_record, run_dir_mode=KEEP_UNLESS_REFUSED)
