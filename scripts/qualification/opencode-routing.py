#!/usr/bin/env python3
"""macOS-only, installed OpenCode 1.18.29 model-selection counterexample.

Uses synthetic fresh profiles and a loopback mock. sandbox-exec denies external
network. Stops after observing primary model selection; this is not an inference
quality, receipt, catalog writer, or export round-trip test. Installs nothing.
"""
import argparse
import hashlib
import http.server
import json
import os
import pathlib
import re
import signal
import subprocess
import tempfile
import threading
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--opencode", required=True, type=pathlib.Path)
args = parser.parse_args()
if subprocess.check_output([str(args.opencode), "--version"], text=True).strip() != "1.18.29":
    raise SystemExit("unqualified OpenCode version")
if not pathlib.Path("/usr/bin/sandbox-exec").is_file():
    raise SystemExit("requires macOS sandbox-exec; refusing an unrestricted fallback")
root = pathlib.Path(tempfile.mkdtemp(prefix="opencode-routing-"))
observed = []


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        if length > 1024 * 1024:
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(length))
        observed.append(body.get("model"))
        # A finite SSE response, with no tool call or external operation.
        chunk = {"id": "fixture", "object": "chat.completion.chunk", "created": 0,
                 "model": body.get("model"), "choices": [{"index": 0,
                 "delta": {"role": "assistant", "content": "Synthetic."}, "finish_reason": None}]}
        end = {**chunk, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}
        data = ("data: " + json.dumps(chunk) + "\n\ndata: " + json.dumps(end) + "\n\ndata: [DONE]\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
profile = root / "sandbox.sb"
profile.write_text('(version 1) (allow default) (deny network*) '
                   '(allow network-outbound (remote ip "localhost:*")) '
                   '(allow network-inbound (local ip "localhost:*"))')


def provider(model):
    return {"npm": "@ai-sdk/openai-compatible", "name": "Synthetic",
            "options": {"baseURL": f"http://127.0.0.1:{server.server_port}/v1", "apiKey": "synthetic-only"},
            "models": {model: {"name": model, "limit": {"context": 4096, "output": 100}}}}


existing = {"existing": provider("existing-model")}
cases = [
    ("fresh-before", {}, None),
    ("fresh-after", {"provider": {"trace-test": provider("trace-model")}}, None),
    ("blank-before", {"model": ""}, None),
    ("blank-after", {"model": "", "provider": {"trace-test": provider("trace-model")}}, None),
    ("explicit-before", {"model": "existing/existing-model", "provider": existing}, None),
    ("explicit-after", {"model": "existing/existing-model", "provider": {**existing, "trace-test": provider("trace-model")}}, None),
    ("recent-after", {"provider": {"trace-test": provider("trace-model"), **existing}},
     {"recent": [{"providerID": "existing", "modelID": "existing-model"}]}),
]
results = []
try:
    for name, config, recent in cases:
        home = root / name
        work = home / "work"
        work.mkdir(parents=True)
        config_path = home / "config/opencode/opencode.json"
        config_path.parent.mkdir(parents=True)
        config_path.write_text(json.dumps({"$schema": "https://opencode.ai/config.json", **config}))
        state = home / "state/opencode/model.json"
        if recent:
            state.parent.mkdir(parents=True)
            state.write_text(json.dumps(recent))
        env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home),
               **{f"XDG_{k}_HOME": str(home / v) for k, v in
                  [("CONFIG", "config"), ("DATA", "data"), ("STATE", "state"), ("CACHE", "cache")]},
               "OPENCODE_DISABLE_AUTOUPDATE": "true", "OPENCODE_DISABLE_MODELS_FETCH": "true",
               "OPENCODE_DISABLE_PROJECT_CONFIG": "true", "OPENCODE_EXPERIMENTAL_DISABLE_FILEWATCHER": "true",
               "NO_PROXY": "*"}
        before = config_path.read_bytes()
        start = len(observed)
        log = home / "run.log"
        with log.open("wb") as output:
            proc = subprocess.Popen(["/usr/bin/sandbox-exec", "-f", str(profile), str(args.opencode),
                                     "--pure", "--print-logs", "run", "--format", "json", "Return the synthetic word."],
                                    cwd=work, env=env, stdout=output, stderr=output, start_new_session=True)
            found = None
            try:
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    found = re.search(r"message=stream providerID=(\S+) modelID=(\S+).*small=false", log.read_text())
                    if found and (name in ("fresh-before", "blank-before") or len(observed) > start):
                        break
                    if proc.poll() is not None:
                        break
                    time.sleep(0.1)
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGTERM)
                    try:
                        proc.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        os.killpg(proc.pid, signal.SIGKILL)
                        proc.wait()
        if not found:
            raise SystemExit(f"model selection not observed: {name}; logs {root}")
        results.append({"case": name, "primary_model": "/".join(found.groups()),
                        "mock_models": sorted(set(observed[start:])),
                        "config_unchanged": before == config_path.read_bytes()})
    by_name = {r["case"]: r for r in results}
    assert by_name["fresh-before"]["primary_model"] != by_name["fresh-after"]["primary_model"]
    assert by_name["fresh-after"]["primary_model"] == "trace-test/trace-model"
    assert by_name["blank-before"]["primary_model"] != by_name["blank-after"]["primary_model"]
    assert by_name["blank-after"]["primary_model"] == "trace-test/trace-model"
    assert by_name["explicit-before"]["primary_model"] == by_name["explicit-after"]["primary_model"] == "existing/existing-model"
    assert by_name["recent-after"]["primary_model"] == "existing/existing-model"
    assert all(r["config_unchanged"] for r in results)
    (root / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({"version": "1.18.29", "binary_sha256": hashlib.sha256(args.opencode.read_bytes()).hexdigest(), "results": results, "catalog_enablement": "REFUSED: fresh default changes", "artifacts": str(root)}, indent=2))
finally:
    server.shutdown()
