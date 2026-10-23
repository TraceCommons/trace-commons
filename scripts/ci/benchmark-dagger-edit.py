#!/usr/bin/env python3
"""Compare warm Dagger build checks after three equivalent Rust body edits."""
import argparse
import datetime as dt
import hashlib
import http.client
import json
import os
from pathlib import Path
import platform
import re
import socket
import signal
import statistics
import subprocess
import time

TARGET = "crates/trace-commons-protocol/src/first_run_estimate.rs"
ORIGINAL = "end_ok(self.min) && end_ok(self.max) && self.min <= self.max"
VARIANTS = [
    "self.min <= self.max && end_ok(self.min) && end_ok(self.max)",
    "end_ok(self.max) && self.min <= self.max && end_ok(self.min)",
    "self.min <= self.max && end_ok(self.max) && end_ok(self.min)",
]
COMMAND = ["dagger", "--progress=plain", "-v", "check", "*:build"]
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def capture(args):
    run = subprocess.run(args, text=True, capture_output=True)
    return run.stdout.strip() if run.returncode == 0 else None


def engine_stats():
    """Docker cgroup counters, not a Dagger Cloud billing measurement."""
    try:
        endpoint = os.environ.get("DOCKER_HOST") or capture([
            "docker", "context", "inspect", "--format", "{{.Endpoints.docker.Host}}",
        ])
        if not endpoint or not endpoint.startswith("unix://"):
            return None
        conn = http.client.HTTPConnection("localhost", timeout=5)
        transport = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        transport.settimeout(5)
        transport.connect(endpoint.removeprefix("unix://"))
        conn.sock = transport
        try:
            conn.request("GET", "/containers/dagger-engine-v0.21.11/stats?stream=false&one-shot=true")
            response = conn.getresponse()
            data = json.loads(response.read())
            if response.status != 200:
                return None
            return {
                "container_id": data["id"],
                "cpu_ns": data["cpu_stats"]["cpu_usage"]["total_usage"],
                "memory_bytes": data.get("memory_stats", {}).get("usage"),
            }
        finally:
            conn.close()
    except (OSError, ValueError, KeyError, http.client.HTTPException):
        return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", required=True)
    parser.add_argument("--output", default=".local/dagger-edit-benchmark")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    os.chdir(root)
    output = Path(args.output).resolve()
    allowed_output = (root / ".local").resolve()
    if not output.is_relative_to(allowed_output):
        parser.error("output must stay under ignored .local/ to avoid changing pipeline inputs")
    output.mkdir(parents=True, exist_ok=True)
    target = root / TARGET
    original = target.read_bytes()
    old_stat = target.stat()
    text = original.decode()
    if text.count(ORIGINAL) != 1:
        raise SystemExit("expected exactly one frozen benchmark edit site")
    revision = os.environ.get("BENCH_REVISION") or capture(["git", "rev-parse", "HEAD"])
    docker_info = capture(["docker", "info", "--format", "{{json .}}"])
    docker_info = json.loads(docker_info) if docker_info else {}
    report = {
        "backend": args.backend,
        "revision": revision,
        "started_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "host_system": platform.system(),
        "host_architecture": platform.machine(),
        "host_cpu_count": os.cpu_count(),
        "docker": {key: docker_info.get(key) for key in ["NCPU", "MemTotal", "Architecture", "ServerVersion", "KernelVersion", "OperatingSystem"]},
        "dagger_version": capture(["dagger", "version"]),
        "module_sha256": digest((root / ".dagger/src/trace_commons_ci/main.py").read_bytes()),
        "cargo_lock_sha256": digest((root / "Cargo.lock").read_bytes()),
        "harness_sha256": digest(Path(__file__).read_bytes()),
        "edit_file": TARGET,
        "original_file_sha256": digest(original),
        "original_expression": ORIGINAL,
        "command": COMMAND,
        "scope": "existing build lane only; same engine persists within each backend run",
        "stages": [],
        "complete": False,
    }

    def save():
        (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")

    def stage(name, expression=None, require_cached=False):
        before = target.read_bytes()
        before_stats = engine_stats()
        log_path = output / f"{name}.log"
        started = time.perf_counter()
        print(f"START {name} backend={args.backend}", flush=True)
        with log_path.open("w") as log:
            result = subprocess.run(COMMAND, stdout=log, stderr=subprocess.STDOUT)
        elapsed = time.perf_counter() - started
        after_stats = engine_stats()
        log_text = ANSI.sub("", log_path.read_text())
        (output / f"{name}.clean.log").write_text(log_text)
        cached = bool(re.search(r"TraceCommonsCi\.build CACHED", log_text))
        rebuilt = re.findall(r"(?:Checking|Compiling)\s+(trace-commons-[\w-]+)\s", log_text)
        protocol_vertices = set()
        execution_states = {}
        for line in log_text.splitlines():
            vertex = re.match(r"\s*(\d+)\s*:", line)
            if not vertex:
                continue
            vertex_id = vertex.group(1)
            if re.search(r"(?:Checking|Compiling)\s+trace-commons-protocol\s", line):
                protocol_vertices.add(vertex_id)
            state = re.search(r"Container\.withExec (DONE|CACHED|ERROR)", line)
            if state:
                execution_states[vertex_id] = state.group(1)
        protocol_execution = {vertex: execution_states.get(vertex) for vertex in sorted(protocol_vertices)}
        item = {
            "name": name,
            "elapsed_seconds": round(elapsed, 3),
            "returncode": result.returncode,
            "file_sha256": digest(before),
            "expression": expression or (VARIANTS[-1] if name == "05-edited-cached" else ORIGINAL),
            "cached_check_result": cached,
            "rebuilt_workspace_crates": sorted(set(rebuilt)),
            "protocol_execution_vertices": protocol_execution,
            "engine_cpu_seconds": None,
        }
        if before_stats and after_stats and before_stats["container_id"] == after_stats["container_id"]:
            item["engine_cpu_seconds"] = round((after_stats["cpu_ns"] - before_stats["cpu_ns"]) / 1e9, 3)
        report["stages"].append(item)
        save()
        if target.read_bytes() != before:
            raise RuntimeError("benchmark source changed during a measured stage")
        if result.returncode:
            raise RuntimeError(f"{name} failed; see {log_path}")
        if expression and "trace-commons-protocol" not in rebuilt:
            raise RuntimeError(f"{name} did not rebuild the edited crate; benchmark invalid")
        if expression and not any(state == "DONE" for state in protocol_execution.values()):
            raise RuntimeError(f"{name} has no uncached Cargo execution rebuilding protocol")
        if expression and cached:
            raise RuntimeError(f"{name} reused the whole check despite a new source edit")
        if require_cached and not cached:
            raise RuntimeError(f"{name} failed to demonstrate unchanged check-result reuse")
        print(f"PASS {name} seconds={elapsed:.3f} cached={cached} rebuilt={item['rebuilt_workspace_crates']}", flush=True)

    def terminate(signum, _frame):
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, terminate)
    save()
    try:
        stage("00-prime")
        stage("01-baseline-cached", require_cached=True)
        for number, expression in enumerate(VARIANTS, 1):
            target.write_text(text.replace(ORIGINAL, expression))
            stage(f"0{number + 1}-edit-{number}", expression)
        stage("05-edited-cached", require_cached=True)
        report["edit_median_seconds"] = statistics.median(
            item["elapsed_seconds"] for item in report["stages"] if re.fullmatch(r"0[234]-edit-[123]", item["name"])
        )
        report["complete"] = True
    except BaseException as error:
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        target.write_bytes(original)
        os.utime(target, ns=(old_stat.st_atime_ns, old_stat.st_mtime_ns))
        report["source_restored"] = digest(target.read_bytes()) == digest(original)
        report["finished_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
        save()
    print(json.dumps({"backend": args.backend, "edit_median_seconds": report["edit_median_seconds"], "results": str(output / "results.json")}), flush=True)


if __name__ == "__main__":
    main()
