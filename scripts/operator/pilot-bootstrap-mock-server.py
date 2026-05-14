#!/usr/bin/env python3
"""
Mock server for the pilot-bootstrap smoke harness.

Plays two roles on the same loopback port (default 127.0.0.1:3907):

  1. **Mock tracedao-ingest**. Accepts POST /v1/traces with a bearer
     token. Each request must carry a JSON body with a top-level
     `submission_id` field (a UUID string, per
     TraceContributionEnvelope). The server records every distinct
     submission_id and the per-id POST count, then returns a 200 of
     shape `{"status":"accepted","submission_id":"..."}` on first
     sight and `{"status":"duplicate",...}` on every repeat. The
     binary's submitter wire format reads `{status}` from the body.

  2. **Mock HuggingFace Hub API**. Implements only the single endpoint
     the binary actually calls during dataset discovery:
     `GET /api/datasets/{repo_id}`, returning a siblings list that
     advertises one parquet file. The smoke pre-populates the hf-hub
     cache directly so no file download ever happens; we therefore do
     not implement /resolve/. If the binary's cache layout assumption
     ever drifts, this mock will surface that as a 404 on /resolve/
     rather than silently succeeding.

  Admin surface (used by the bash smoke):

     GET  /__smoke/stats   -> {"distinct": N, "total": M, "ids": [...]}
     POST /__smoke/reset   -> clears stats; returns {"ok": true}

  Admin paths require no auth — this server is bound to loopback only
  and exists only for operator-local smoke runs. Do not point it at
  any network-reachable interface.

  Stdlib only: http.server + json. SIGTERM/SIGINT exits cleanly.
"""

from __future__ import annotations

import argparse
import json
import signal
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class MockState:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.counts: dict[str, int] = {}
        self.total = 0

    def record(self, submission_id: str) -> tuple[str, int]:
        with self.lock:
            self.total += 1
            count = self.counts.get(submission_id, 0) + 1
            self.counts[submission_id] = count
            decision = "accepted" if count == 1 else "duplicate"
            return decision, count

    def snapshot(self) -> dict:
        with self.lock:
            return {
                "distinct": len(self.counts),
                "total": self.total,
                "ids": sorted(self.counts.keys()),
            }

    def reset(self) -> None:
        with self.lock:
            self.counts.clear()
            self.total = 0


STATE = MockState()

# Set in main() — read by Handler.do_GET via globals so we don't subclass
# BaseHTTPRequestHandler with a custom __init__.
HF_DATASET = ""
HF_COMMIT = ""
HF_SIBLINGS: list[str] = []


def _log(msg: str) -> None:
    print(f"mock-ingest: {msg}", file=sys.stderr, flush=True)


class Handler(BaseHTTPRequestHandler):
    # Silence the default per-request access log; we emit our own.
    def log_message(self, fmt: str, *args) -> None:  # noqa: A003
        return

    def _send_json(self, code: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/__smoke/stats":
            self._send_json(200, STATE.snapshot())
            return

        # HF Hub API: GET /api/datasets/{repo_id}[/revision/{rev}]
        hf_prefix = "/api/datasets/"
        if self.path.startswith(hf_prefix):
            tail = self.path[len(hf_prefix):]
            # Strip an optional `/revision/{name}` suffix; hf-hub appends it
            # when a revision is specified (defaults to `main`).
            if "/revision/" in tail:
                repo_id = tail.split("/revision/", 1)[0]
            else:
                repo_id = tail
            if repo_id != HF_DATASET:
                _log(f"hf-api: unknown dataset {repo_id}")
                self._send_json(404, {"error": "RepoNotFound"})
                return
            payload = {
                "id": HF_DATASET,
                "sha": HF_COMMIT,
                "siblings": [{"rfilename": name} for name in HF_SIBLINGS],
            }
            _log(f"hf-api: info {HF_DATASET} -> {len(HF_SIBLINGS)} siblings")
            self._send_json(200, payload)
            return

        self._send_json(404, {"error": "not_found"})

    def do_POST(self) -> None:  # noqa: N802
        if self.path == "/__smoke/reset":
            STATE.reset()
            _log("state reset")
            self._send_json(200, {"ok": True})
            return

        if self.path != "/v1/traces":
            self._send_json(404, {"error": "not_found"})
            return

        auth = self.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            _log("rejected: missing bearer")
            self._send_json(401, {"error": "unauthorized"})
            return

        length = int(self.headers.get("Content-Length", "0") or "0")
        if length <= 0:
            _log("rejected: empty body")
            self._send_json(400, {"error": "empty_body"})
            return

        raw = self.rfile.read(length)
        try:
            envelope = json.loads(raw)
        except json.JSONDecodeError as exc:
            _log(f"rejected: invalid json ({exc})")
            self._send_json(400, {"error": "invalid_json"})
            return

        submission_id = envelope.get("submission_id")
        if not isinstance(submission_id, str) or not submission_id:
            _log("rejected: missing submission_id")
            self._send_json(400, {"error": "missing_submission_id"})
            return

        decision, count = STATE.record(submission_id)
        token_len = len(auth) - len("Bearer ")
        _log(
            f"POST /v1/traces submission_id={submission_id} "
            f"decision={decision} count={count} token_len={token_len}"
        )
        self._send_json(
            200,
            {"status": decision, "submission_id": submission_id},
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=3907)
    parser.add_argument(
        "--hf-dataset",
        default="",
        help="HF dataset id to advertise via /api/datasets/{id}.",
    )
    parser.add_argument(
        "--hf-commit",
        default="smokecommit0000000000000000000000000001",
        help="Pseudo commit hash returned as `sha` in the HF info response.",
    )
    parser.add_argument(
        "--hf-sibling",
        action="append",
        default=[],
        help="Repeatable: rfilename entries to advertise in siblings.",
    )
    args = parser.parse_args()

    if args.host not in ("127.0.0.1", "localhost", "::1"):
        print(
            f"mock-ingest: refusing to bind non-loopback host {args.host}",
            file=sys.stderr,
        )
        return 2

    global HF_DATASET, HF_COMMIT, HF_SIBLINGS
    HF_DATASET = args.hf_dataset
    HF_COMMIT = args.hf_commit
    HF_SIBLINGS = list(args.hf_sibling)

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    _log(f"listening on {args.host}:{args.port}")
    if HF_DATASET:
        _log(
            f"hf-api: advertising dataset={HF_DATASET} "
            f"commit={HF_COMMIT} siblings={HF_SIBLINGS}"
        )

    stop = threading.Event()

    def shutdown(signum, _frame):
        _log(f"signal {signum} received; shutting down")
        stop.set()
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)

    try:
        server.serve_forever()
    finally:
        server.server_close()
        _log("server closed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
