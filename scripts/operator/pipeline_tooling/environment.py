"""Run and Environment: the child-process boundary for pipeline tooling.

`_invoke` is the one place a Docker, cargo, or psql child process starts.
Every such call in this module goes through it, so the self-tests can
replace the whole surface with a single monkeypatch and need no Docker and
no cargo. `_code_revision_hash`'s `git ls-files` call is the exception: it
builds the tree hash at `Run.create()` time and again when a command that
credits evidence to that hash finishes
(`Run.require_code_revision_unchanged`), so it calls `subprocess` on its
own. The self-tests construct `Run` directly and replace
`_code_revision_hash` where a command reaches its end.
"""

from __future__ import annotations

import hashlib
import os
import re
import secrets
import subprocess
import time
import tomllib
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlsplit

from .errors import StepFailed, ToolingError, require

# scripts/operator/pipeline_tooling/environment.py -> repo root.
ROOT = Path(__file__).resolve().parents[3]

# Pinned by digest so every run uses the same bytes. Confirmed against the
# locally pulled `postgres:16` image with
# `docker image inspect postgres:16 --format '{{index .RepoDigests 0}}'`
# (P4-D6); re-check with `docker buildx imagetools inspect postgres:16` and
# update this constant if the published index digest ever moves.
POSTGRES_IMAGE = "postgres@sha256:a3b7f434b2dc57ce85a67e171163eb8ab1a1ebcb39d27484661f26b1dfbe30d6"

# Only these ambient variables cross into a child process unmodified. Every
# other TRACE_COMMONS_* value a caller wants a child to see must be passed
# explicitly to `child_environment`, and every other ambient variable
# (DATABASE_URL, PGPASSWORD, cloud credentials, tenant tokens, ...) is
# dropped even if it happens to be set in this process's own environment.
CHILD_ENV_ALLOWLIST = (
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "CARGO_TERM_COLOR",
    "CARGO_INCREMENTAL",
    "RUSTFLAGS",
    "SystemRoot",
)

_STEP_LABEL = re.compile(r"[a-z0-9_]{1,64}\Z")
_DATABASE_NAME = re.compile(r"[a-z][a-z0-9_]{0,62}\Z")
_DUMP_NAME = re.compile(r"[a-z0-9_]{1,64}\.dump\Z")
_EXCLUDED_TREE_DIRS = {".local", ".vscode", "target"}


def child_environment(extra):
    """The environment a child process receives: the allowlisted ambient
    variables plus `extra`, whose keys must all be TRACE_COMMONS_* (the
    tooling variables named in the plan)."""
    result = {}
    for key in CHILD_ENV_ALLOWLIST:
        if key in os.environ:
            result[key] = os.environ[key]
    for key, value in extra.items():
        require(key.startswith("TRACE_COMMONS_"), "child_environment_key_invalid")
        result[key] = value
    return result


def _invoke(command, *, env, capture=False, input_text=None, log_path=None):
    """The sole child-process boundary. Tests monkeypatch this function
    itself to avoid Docker, cargo, and psql."""
    if capture:
        completed = subprocess.run(
            list(command),
            env=env,
            input=input_text,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            check=False,
        )
        return completed.returncode, completed.stdout
    with open(log_path, "ab") as handle:
        completed = subprocess.run(
            list(command),
            env=env,
            input=input_text.encode() if input_text is not None else None,
            stdout=handle,
            stderr=subprocess.STDOUT,
            check=False,
        )
    return completed.returncode, None


# The official postgres image runs a transient, unix-socket-only server to
# execute init scripts, stops it, then starts the real one; `pg_isready` can
# report ready against the transient server moments before that restart, so
# the very next psql connection can land in the gap and see no socket at
# all. Retrying briefly bridges that window without masking a real SQL
# error, which never matches these connection-refused shapes.
_CONNECTION_RETRY_MARKERS = (
    "could not connect",
    "No such file or directory",
    "Connection refused",
    "the database system is starting up",
    "server closed the connection unexpectedly",
)


def _invoke_psql(command, *, env, input_text):
    result = (1, "")
    for _ in range(40):
        returncode, output = _invoke(command, env=env, capture=True, input_text=input_text)
        if returncode == 0 or not any(marker in (output or "") for marker in _CONNECTION_RETRY_MARKERS):
            return returncode, output
        result = (returncode, output)
        time.sleep(0.25)
    return result


# What the code revision covers besides the server crate's dependency
# closure (#1249). Each entry is a file, or a directory and everything under
# it, relative to the repository root; an entry that does not exist covers
# nothing until it does.
#
# - The workspace manifest and lockfile, a toolchain pin, and a checked-in
#   cargo configuration decide how every covered crate builds.
# - `cloudbuild.yaml` names the features the deployed binaries are built
#   with.
# - `migrations/` is the schema the binary applies at start.
# - The rest is what a qualification run reads: its tooling, the contract
#   manifest, the default corpus, the package digest vector that the
#   qualification module's tests and the self-test both read, and the
#   top-level `.gitignore`, which the self-test reads and which decides the
#   files listed here (`CodeRevisionRepositoryTests` checks that every input
#   the tooling names is covered).
CODE_REVISION_ROOT_CRATE = "crates/trace-commons-server"
CODE_REVISION_COVERED = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain",
    "rust-toolchain.toml",
    ".cargo",
    "cloudbuild.yaml",
    "migrations",
    "scripts/operator/pipeline.py",
    "scripts/operator/pipeline_tooling",
    "scripts/operator/pipeline-deployment-inventory.py",
    "scripts/operator/test_pipeline_tooling.py",
    "scripts/operator/fixtures/pipeline-package-digests-vector.json",
    "docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json",
    "docs/superpowers/specs/fixtures",
    ".gitignore",
)

# The dependency tables whose `path` entries are part of a crate's build. A
# `[dev-dependencies]` table is not one of them (#1249, an owner decision): a
# dev-dependency is compiled into the test binaries a qualification run
# builds, but never into the server binaries the revision identifies. So a
# change to one (the contributor crate) can change what a qualification run
# finds, a compile failure for one, and keep the revision; it cannot change
# what the deployed server does. Adding "dev-dependencies" here for the root
# crate alone would cover it, and would requalify on every contributor
# change.
_BUILD_DEPENDENCY_TABLES = ("dependencies", "build-dependencies")

# `include_str!`, `include_bytes!` and `include!`. An argument is a string
# literal, resolved against the including file's directory, or
# `concat!(env!("CARGO_MANIFEST_DIR"), "<literal>")`, resolved against the
# crate's. An occurrence followed by `\"` is text inside a string literal,
# not a call. Any other argument is refused: the file it reads is unknown.
_INCLUDE_CALL = re.compile(r"\binclude(?:_str|_bytes)?!\s*\(")
_INCLUDE_LITERAL = re.compile(r'\binclude(?:_str|_bytes)?!\s*\(\s*"([^"\\]*)"\s*,?\s*\)')
_INCLUDE_MANIFEST_DIR = re.compile(
    r'\binclude(?:_str|_bytes)?!\s*\(\s*concat!\s*\(\s*env!\s*\(\s*"CARGO_MANIFEST_DIR"\s*\)\s*,'
    r'\s*"([^"\\]*)"\s*,?\s*\)\s*,?\s*\)'
)
_INCLUDE_IN_STRING = re.compile(r'\binclude(?:_str|_bytes)?!\s*\(\s*\\"')
# `#[path = "..."]`, and `#[cfg_attr(<cfg>, path = "...")]`, which loads a
# module file the same way when its condition holds.
_PATH_ATTRIBUTE = re.compile(r'#\s*\[\s*(?:cfg_attr\s*\([^\]]*?\bpath|path)\s*=\s*"([^"\\]*)"')
# An out-of-line module declaration, `mod name;`, with any attributes and
# visibility. In a Rust file outside the covered crates its file is not
# resolved by the scan, so one is refused (`code_revision_include_unresolved`).
_OUT_OF_LINE_MODULE = re.compile(
    r"(?m)^\s*(?:#\s*\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*;"
)


def _tree_relative(base, relative):
    """`relative`, resolved against the directory `base`, as a normalized
    `/`-separated path; both relative to the repository root. Refused
    (`code_revision_path_outside_tree`) when it leaves the tree."""
    joined = os.path.normpath(os.path.join(base, relative))
    require(
        not os.path.isabs(relative) and joined != ".." and not joined.startswith(".." + os.sep),
        "code_revision_path_outside_tree",
    )
    return Path(joined).as_posix()


def _read_manifest(root, relative):
    path = root / relative
    require(path.is_file(), "code_revision_manifest_missing")
    try:
        return tomllib.loads(path.read_text())
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as error:
        raise ToolingError("code_revision_manifest_invalid") from error


def _path_dependencies(manifest, crate, workspace_dependencies):
    """The crate directories `manifest` (of the crate at `crate`) names
    through `path`, in its build dependency tables, for every target."""
    tables = [manifest.get(name, {}) for name in _BUILD_DEPENDENCY_TABLES]
    for target in manifest.get("target", {}).values():
        tables.extend(target.get(name, {}) for name in _BUILD_DEPENDENCY_TABLES)
    found = []
    for table in tables:
        for name, spec in table.items():
            if not isinstance(spec, dict):
                continue
            if spec.get("workspace") is True:
                inherited = workspace_dependencies.get(name)
                if isinstance(inherited, dict) and "path" in inherited:
                    found.append(_tree_relative(".", inherited["path"]))
            elif "path" in spec:
                found.append(_tree_relative(crate, spec["path"]))
    return found


def _code_revision_crates(root):
    """The crate directories the server's binaries are built from: the
    server crate and, transitively, every crate its build dependency tables
    reach by `path` (a workspace-inherited one included), plus any crate the
    workspace's `[patch]` or `[replace]` tables substitute by `path`. Sorted,
    relative to `root`. A crate without a readable `Cargo.toml` is refused
    (`code_revision_manifest_missing`)."""
    workspace = _read_manifest(root, "Cargo.toml")
    workspace_dependencies = workspace.get("workspace", {}).get("dependencies", {})
    pending = [CODE_REVISION_ROOT_CRATE]
    for source in workspace.get("patch", {}).values():
        pending.extend(
            _tree_relative(".", spec["path"])
            for spec in source.values()
            if isinstance(spec, dict) and "path" in spec
        )
    pending.extend(
        _tree_relative(".", spec["path"])
        for spec in workspace.get("replace", {}).values()
        if isinstance(spec, dict) and "path" in spec
    )
    crates = set()
    while pending:
        crate = pending.pop()
        if crate in crates:
            continue
        crates.add(crate)
        manifest = _read_manifest(root, f"{crate}/Cargo.toml")
        pending.extend(_path_dependencies(manifest, crate, workspace_dependencies))
    return sorted(crates)


def _included_paths(root, crate, source):
    """The files the Rust source `source` (relative to `root`, in the crate
    at `crate`) includes or loads as a module by `#[path]`, relative to
    `root`, each paired with whether it is Rust source (`include!` and
    `#[path]`; not `include_str!` or `include_bytes!`)."""
    text = (root / source).read_text(errors="replace")
    directory = os.path.dirname(source)
    found = []
    for call in _INCLUDE_CALL.finditer(text):
        if _INCLUDE_IN_STRING.match(text, call.start()):
            continue
        is_source = text.startswith("include!", call.start())
        literal = _INCLUDE_LITERAL.match(text, call.start())
        if literal:
            found.append((_tree_relative(directory, literal.group(1)), is_source))
            continue
        manifest_relative = _INCLUDE_MANIFEST_DIR.match(text, call.start())
        require(manifest_relative is not None, "code_revision_include_unresolved")
        found.append((_tree_relative(crate, manifest_relative.group(1).lstrip("/")), is_source))
    for attribute in _PATH_ATTRIBUTE.finditer(text):
        found.append((_tree_relative(directory, attribute.group(1)), True))
    return found


def _listed_files():
    """Every tracked-or-untracked, non-ignored file of the checkout, relative
    to `ROOT`, as `git` lists it (see `_code_revision_hash`)."""
    listing = subprocess.run(
        [
            "git",
            "-c",
            "core.excludesFile=",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-per-directory=.gitignore",
            "--exclude=.cargo/",
            "-z",
        ],
        check=True,
        capture_output=True,
        cwd=ROOT,
    )
    files = set()
    for raw in listing.stdout.split(b"\0"):
        if not raw:
            continue
        path = Path(raw.decode())
        if path.parts and path.parts[0] in _EXCLUDED_TREE_DIRS:
            continue
        if (ROOT / path).is_file():
            files.add(path.as_posix())
    return files


def _code_revision_paths():
    """The files the code revision hashes, sorted (#1249): of the listed
    files, those under a crate of `_code_revision_crates`, under an entry of
    `CODE_REVISION_COVERED`, or included (`include_str!`, `include_bytes!`,
    `include!`, `#[path]`, `#[cfg_attr(..., path = ...)]`) by a Rust file of
    a covered crate, or, transitively, by a Rust file that one of those
    loads by `include!` or `#[path]`. An included file outside those
    prefixes that is not listed is refused (`code_revision_include_missing`):
    its bytes would otherwise be left out of the hash unseen. So is an
    out-of-line `mod name;` in a loaded Rust file outside the covered crates
    (`code_revision_include_unresolved`): the scan does not resolve its
    file. The scan reads comments too, so a comment naming such a file adds
    it, which only widens the revision."""
    files = _listed_files()
    prefixes = [*_code_revision_crates(ROOT), *CODE_REVISION_COVERED]

    def covered(path):
        return any(path == prefix or path.startswith(prefix + "/") for prefix in prefixes)

    crates = _code_revision_crates(ROOT)
    selected = {path for path in files if covered(path)}
    # Each Rust file of a covered crate, then, transitively, each file an
    # `include!` or `#[path]` loads as Rust source, wherever it is: what that
    # file includes is compiled into the same crate. `CARGO_MANIFEST_DIR` is
    # the including crate's throughout.
    pending = [
        (source, crate)
        for crate in crates
        for source in sorted(path for path in selected if path.startswith(crate + "/") and path.endswith(".rs"))
    ]
    scanned = {source for source, _ in pending}
    while pending:
        source, crate = pending.pop()
        if not any(source.startswith(other + "/") for other in crates):
            # A module file outside every covered crate: a child module it
            # declares out of line lives in a file the scan does not
            # resolve, so it is refused rather than left out.
            text = (ROOT / source).read_text(errors="replace")
            require(_OUT_OF_LINE_MODULE.search(text) is None, "code_revision_include_unresolved")
        for included, is_source in _included_paths(ROOT, crate, source):
            if not covered(included):
                require(included in files, "code_revision_include_missing")
                selected.add(included)
            # A target under a covered prefix that is not listed is not
            # there, and so cannot change what is built.
            if is_source and included in files and included not in scanned:
                scanned.add(included)
                pending.append((included, crate))
    return sorted(selected, key=lambda path: path.encode())


def _code_revision_hash():
    """The code revision: the length-prefixed path and content of each file
    of `_code_revision_paths()`, in sorted path order (#1249 narrowed the
    set of files; the framing is the one ported from `ef97a459:scripts/
    operator/run-pipeline-qualification.sh` lines 70-88). A file outside
    that set, a document or a client shell, does not change it.

    The files are taken from git's listing: every tracked-or-untracked,
    non-ignored file, excluding the top-level `.local`, `.vscode`, and
    `target` directories.

    "Non-ignored" means not ignored by the repository's own `.gitignore`
    files, and nothing else: `--exclude-per-directory=.gitignore` in place of
    `--exclude-standard`, which also applies the host's `.git/info/exclude`
    and the user's global excludes file (`core.excludesFile`, emptied here as
    well). The server compares the revision a release was built with to the
    revision of the run that qualified it, so one checkout must give one
    revision on every host, and a file that only a host's own exclude list
    hides is part of the tree.

    One exception: an untracked `.cargo/` directory, at any depth, is left
    out (`--exclude=.cargo/`). It holds a developer's local cargo
    configuration (a job count, a target directory), and the repository's
    `.gitignore` does not list it. An ignore rule does not apply to a
    tracked file; a line there would keep a new `.cargo/config.toml`, not
    yet added, out of `git status` and out of `git add`, so a cargo
    configuration that was meant to be checked in could be left out of a
    commit unseen. `--exclude` applies to untracked files only: a tracked
    file under `.cargo/` is listed by `--cached` and is part of the revision,
    so a checked-in cargo configuration, which changes how the code builds,
    changes the revision.
    With no `.cargo` directory in the checkout the revision is what it was
    before this exception."""
    tree = hashlib.sha256()
    for path in _code_revision_paths():
        raw_path = path.encode()
        tree.update(len(raw_path).to_bytes(8, "big"))
        tree.update(raw_path)
        content = (ROOT / path).read_bytes()
        tree.update(len(content).to_bytes(8, "big"))
        tree.update(content)
    return "sha256:" + tree.hexdigest()


@dataclass
class Run:
    run_id: str
    started_at: datetime
    run_dir: Path
    code_revision_hash: str
    cleanup_failed: bool = False

    @classmethod
    def create(cls):
        run_id = "q" + secrets.token_hex(4)
        run_dir = ROOT / ".local" / "pipeline" / "runs" / run_id
        run_dir.mkdir(parents=True, exist_ok=False)
        run_dir.chmod(0o700)
        (run_dir / "logs").mkdir()
        (run_dir / "results").mkdir()
        (run_dir / "artifacts").mkdir()
        return cls(
            run_id=run_id,
            started_at=datetime.now(timezone.utc),
            run_dir=run_dir,
            code_revision_hash=_code_revision_hash(),
        )

    def require_code_revision_unchanged(self):
        """The tree hash again, at the end of a command that credits its
        results and report to `code_revision_hash`: a file edited while the
        command ran would otherwise be credited to the tree it started from
        (Zaki's review of #1166, minor 1). A mismatch is
        `code_revision_changed`."""
        require(_code_revision_hash() == self.code_revision_hash, "code_revision_changed")

    def log_path(self, step):
        require(_STEP_LABEL.fullmatch(step) is not None, "step_label_invalid")
        path = self.run_dir / "logs" / f"{step}.log"
        path.touch(exist_ok=True)
        path.chmod(0o600)
        return path

    @property
    def results_dir(self):
        return self.run_dir / "results"


def run_child(run, step, command, env):
    log_path = run.log_path(step)
    returncode, _ = _invoke(command, env=env, log_path=log_path)
    if returncode != 0:
        raise StepFailed(step, returncode, log_path)


@dataclass(frozen=True)
class Scenario:
    _environment: "Environment" = field(repr=False)
    runtime_database: str
    upgrade_database: str
    pilot_database: str
    host: str
    port: int
    artifact_root: Path

    @property
    def runtime_url(self):
        return f"postgres://trace@{self.host}:{self.port}/{self.runtime_database}"

    @property
    def upgrade_url(self):
        return f"postgres://trace@{self.host}:{self.port}/{self.upgrade_database}"

    @property
    def login_resolver_url(self):
        return (
            f"postgres://tc_login_resolver_login@{self.host}:{self.port}"
            f"/{self.runtime_database}"
        )

    @property
    def restored_database(self):
        """Where `restore-drill` restores `pilot_database`'s dump: a sibling
        in the same cluster, dropped with the scenario."""
        return f"{self.runtime_database}_restored"

    @property
    def restored_url(self):
        return f"postgres://trace@{self.host}:{self.port}/{self.restored_database}"

    def committed_transactions(self, *databases):
        in_list = ", ".join(f"'{name}'" for name in databases)
        output = self._environment._psql(
            f"SELECT COALESCE(SUM(xact_commit), 0) FROM pg_stat_database "
            f"WHERE datname IN ({in_list});"
        )
        return int((output or "0").strip() or "0")


_LOCK_DATABASE = "pipeline_tooling_lock"
_LOGIN_RESOLVER_ROLES_SQL = (
    "DO $$\n"
    "BEGIN\n"
    "  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_login_resolver') THEN\n"
    "    CREATE ROLE trace_login_resolver NOLOGIN NOBYPASSRLS;\n"
    "  END IF;\n"
    "  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_login_resolver_login') THEN\n"
    "    CREATE ROLE tc_login_resolver_login LOGIN NOBYPASSRLS;\n"
    "  END IF;\n"
    "END\n"
    "$$;\n"
    "GRANT trace_login_resolver TO tc_login_resolver_login;\n"
)


class Environment:
    """A PostgreSQL server for pipeline checks to run against: either a
    throwaway container this process starts and removes, or an existing
    server named by `--postgres-admin-url`, held for the run's duration by
    a lock database. Context manager: `with Environment(run, ...) as env:`.
    """

    def __init__(self, run, *, postgres_admin_url=None):
        self._run = run
        self._postgres_admin_url = postgres_admin_url
        self._container = None
        self._host = None
        self._port = None
        self._lock_acquired = False
        self._scenarios = []
        self._scenario_counter = 0

    # -- context manager -------------------------------------------------

    def __enter__(self):
        # A failure partway through setup (the readiness wait, the port
        # read, the login-resolver SQL, ...) must not leak whatever was
        # already created: Python never calls `__exit__` when `__enter__`
        # raises, so the same teardown `__exit__` runs has to run here too,
        # before the original error propagates. `_teardown` is itself safe
        # to call no matter how little setup completed (see its callees).
        try:
            if self._postgres_admin_url:
                self._enter_admin_mode()
            else:
                self._enter_container_mode()
            self._psql(_LOGIN_RESOLVER_ROLES_SQL, step="environment_setup")
        except BaseException:
            self._teardown()
            raise
        return self

    def __exit__(self, exc_type, exc, tb):
        self._teardown()
        return False

    def _teardown(self):
        """Every cleanup step, each run even when an earlier one raised. A
        step that raises (for example `OSError` when the docker or psql
        binary is gone) sets `cleanup_failed` instead of propagating, so
        teardown never replaces the failure that caused it, and `__exit__`
        and `__enter__` re-raise that original failure."""
        steps = [lambda scenario=scenario: self._drop_scenario_databases(scenario) for scenario in self._scenarios]
        if self._postgres_admin_url:
            # Only drop the lock database if this Environment is the one
            # that created it. `_enter_admin_mode` raising
            # `pipeline_tooling_server_busy` means someone else's lock
            # database already exists; dropping it here would release a
            # different, still-running process's lock.
            if self._lock_acquired:
                steps.append(self._exit_admin_mode)
        else:
            steps.append(self._exit_container_mode)
        for step in steps:
            try:
                step()
            except Exception:  # noqa: BLE001 -- the primary failure must win
                self._run.cleanup_failed = True

    # -- admin-url mode ----------------------------------------------------

    def _enter_admin_mode(self):
        parsed = urlsplit(self._postgres_admin_url)
        require(parsed.hostname == "127.0.0.1", "pipeline_tooling_admin_url_invalid")
        require(parsed.username == "trace", "pipeline_tooling_admin_url_invalid")
        self._host = "127.0.0.1"
        self._port = parsed.port or 5432
        returncode, _ = _invoke_psql(
            ["psql", self._url_for_database("postgres"), "-v", "ON_ERROR_STOP=1", "-qtA"],
            env=child_environment({}),
            input_text=f"CREATE DATABASE {_LOCK_DATABASE};\n",
        )
        if returncode != 0:
            raise ToolingError("pipeline_tooling_server_busy")
        self._lock_acquired = True

    def _exit_admin_mode(self):
        returncode, _ = _invoke_psql(
            ["psql", self._url_for_database("postgres"), "-v", "ON_ERROR_STOP=1", "-qtA"],
            env=child_environment({}),
            input_text=f"DROP DATABASE IF EXISTS {_LOCK_DATABASE};\n",
        )
        if returncode != 0:
            self._run.cleanup_failed = True

    # -- container mode ----------------------------------------------------

    def _enter_container_mode(self):
        name = f"tc-pipeline-{self._run.run_id}"
        # Named before `docker run`, so teardown removes (and checks for) a
        # container that `docker run` created and then failed to start (a
        # port bind failure, for example), not only one that started.
        self._container = name
        returncode, _ = _invoke(
            [
                "docker",
                "run",
                "--detach",
                "--rm",
                "--name",
                name,
                "-e",
                "POSTGRES_USER=trace",
                "-e",
                "POSTGRES_HOST_AUTH_METHOD=trust",
                "-p",
                "127.0.0.1::5432",
                POSTGRES_IMAGE,
            ],
            env=child_environment({}),
            capture=True,
        )
        require(returncode == 0, "pipeline_tooling_container_start_failed")

        # `-h 127.0.0.1 -p 5432` probes TCP on the container's own loopback,
        # not the default unix socket. The official postgres image runs a
        # transient, unix-socket-only server (`listen_addresses=''`) to
        # execute init scripts, then stops it and starts the real,
        # TCP-listening one; a socket-only probe can report ready against
        # that transient server moments before it shuts down mid-restart.
        # Measured locally: a socket probe's very next SQL call failed 6 of
        # 12 times ("FATAL: the database system is shutting down" /
        # "server closed the connection unexpectedly"); the same TCP probe
        # was 0 of 12. The temporary server never listens on TCP at all, so
        # a successful TCP probe can only mean the final server is up.
        ready = False
        for _ in range(60):
            returncode, _ = _invoke(
                ["docker", "exec", name, "pg_isready", "-h", "127.0.0.1", "-p", "5432", "-U", "trace"],
                env=child_environment({}),
                capture=True,
            )
            if returncode == 0:
                ready = True
                break
            time.sleep(0.25)
        require(ready, "pipeline_tooling_container_not_ready")

        returncode, output = _invoke(
            ["docker", "port", name, "5432/tcp"],
            env=child_environment({}),
            capture=True,
        )
        require(returncode == 0, "pipeline_tooling_container_port_unavailable")
        match = re.search(r"127\.0\.0\.1:(\d+)", output or "")
        require(match is not None, "pipeline_tooling_container_port_unavailable")
        self._host = "127.0.0.1"
        self._port = int(match.group(1))

    def _exit_container_mode(self):
        name = self._container
        if name is None:
            return
        # `docker rm -f` itself is not the check: the container may already
        # be gone (crashed, or removed out from under this process), and
        # that is not a cleanup failure. `docker ps -a` confirming nothing
        # by this name remains is the actual proof.
        _invoke(["docker", "rm", "-f", name], env=child_environment({}), capture=True)
        _, remaining = _invoke(
            ["docker", "ps", "-a", "--filter", f"name={name}", "-q"],
            env=child_environment({}),
            capture=True,
        )
        if (remaining or "").strip():
            self._run.cleanup_failed = True

    # -- shared -------------------------------------------------------------

    def _url_for_database(self, database):
        return f"postgres://trace@{self._host}:{self._port}/{database}"

    def _psql_command(self, database="postgres"):
        if self._container is not None:
            return [
                "docker",
                "exec",
                "-i",
                self._container,
                "psql",
                "-v",
                "ON_ERROR_STOP=1",
                "-U",
                "trace",
                "-qtA",
                "-d",
                database,
            ]
        return ["psql", self._url_for_database(database), "-v", "ON_ERROR_STOP=1", "-qtA"]

    def _psql(self, sql, *, database="postgres", step=None):
        """Runs `sql` (through the connection-retry wrapper) and returns its
        output. On failure: with a `step` label, writes the combined output
        to that step's protected log (the same `log_path` helper
        `run_child` uses) and raises `StepFailed`, so `main` reports the
        step, the exit code, and the log path -- never the output itself.
        Without one (a call with no natural step, such as a post-test
        check), raises the plain, label-only `pipeline_tooling_sql_failed`
        as before."""
        returncode, output = _invoke_psql(
            self._psql_command(database), env=child_environment({}), input_text=sql
        )
        if returncode != 0:
            if step is not None:
                log_path = self._run.log_path(step)
                log_path.write_text(output or "")
                raise StepFailed(step, returncode, log_path)
            raise ToolingError("pipeline_tooling_sql_failed")
        return output

    def scenario(self, label):
        require(_STEP_LABEL.fullmatch(label) is not None, "pipeline_tooling_scenario_label_invalid")
        self._scenario_counter += 1
        counter = f"{self._scenario_counter:02d}"
        run8 = self._run.run_id[1:]
        runtime_database = f"admission_test_{run8}_{counter}"
        upgrade_database = f"pipeline_test_{run8}_{counter}"
        self._psql(f"CREATE DATABASE {runtime_database};", step=label)
        self._psql(f"CREATE DATABASE {upgrade_database};", step=label)
        artifact_root = self._run.run_dir / "artifacts" / counter
        artifact_root.mkdir(parents=True, exist_ok=True)
        scenario = Scenario(
            _environment=self,
            runtime_database=runtime_database,
            upgrade_database=upgrade_database,
            pilot_database=f"{runtime_database}_pilot",
            host=self._host,
            port=self._port,
            artifact_root=artifact_root,
        )
        self._scenarios.append(scenario)
        return scenario

    # -- dump and restore ---------------------------------------------------
    #
    # Both stay in this environment's own cluster: the roles the dump's
    # grants name exist there, so the restore keeps ownership and privileges
    # (no `--no-owner`, no `--no-privileges`). Container mode keeps the dump
    # inside the container (`/tmp/<name>`); admin-url mode writes it to
    # `path`, created with mode 0600 before `pg_dump` opens it.

    def _cluster_tool(self, tool):
        if self._container is not None:
            return ["docker", "exec", self._container, tool, "-U", "trace"]
        return [tool, "-h", "127.0.0.1", "-p", str(self._port), "-U", "trace"]

    def _dump_location(self, path):
        path = Path(path)
        require(_DUMP_NAME.fullmatch(path.name) is not None, "pipeline_tooling_dump_name_invalid")
        return f"/tmp/{path.name}" if self._container is not None else str(path)

    def dump(self, database, path):
        require(_DATABASE_NAME.fullmatch(database) is not None, "pipeline_tooling_database_name_invalid")
        location = self._dump_location(path)
        if self._container is None:
            descriptor = os.open(path, os.O_CREAT | os.O_WRONLY | os.O_TRUNC, 0o600)
            os.close(descriptor)
            Path(path).chmod(0o600)
        command = [*self._cluster_tool("pg_dump"), "-Fc", "-f", location, database]
        run_child(self._run, "database_dump", command, child_environment({}))

    def create_database(self, database):
        require(_DATABASE_NAME.fullmatch(database) is not None, "pipeline_tooling_database_name_invalid")
        self._psql(f"CREATE DATABASE {database};", step="database_create")

    def restore(self, path, database):
        require(_DATABASE_NAME.fullmatch(database) is not None, "pipeline_tooling_database_name_invalid")
        command = [*self._cluster_tool("pg_restore"), "-d", database, self._dump_location(path)]
        run_child(self._run, "database_restore", command, child_environment({}))

    def _drop_scenario_databases(self, scenario):
        names = (
            scenario.runtime_database,
            scenario.pilot_database,
            scenario.restored_database,
            scenario.upgrade_database,
        )
        statements = "".join(f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE);\n' for name in names)
        returncode, _ = _invoke_psql(
            self._psql_command(), env=child_environment({}), input_text=statements
        )
        if returncode != 0:
            self._run.cleanup_failed = True
