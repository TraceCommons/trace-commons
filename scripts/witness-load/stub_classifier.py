#!/usr/bin/env python3
"""A stand-in for NEAR AI's `privacy/classify`, for local witness load tests.

LOOPBACK ONLY, SYNTHETIC ONLY. It binds 127.0.0.1, takes any bearer token, and
never forwards anything. Point the witness at it with

  TRACE_PRIVACY_FILTER_BACKEND=near-ai
  TRACE_NEAR_AI_PRIVACY_BASE_URL=http://127.0.0.1:<port>/v1
  TRACE_NEAR_AI_PRIVACY_API_KEY=stub-not-a-key

What it models, and from what measurement:

- Latency per request: `--base-ms + --per-kib-ms * body_KiB`. The defaults fit
  the live endpoint as measured 2026-09-03 (6.4 KB in 1.5 s, 203 KB in 7.1 s,
  243 KiB in 9.3 s): about 1.3 s fixed plus 32 ms per KiB.
- Or, with `--curve measured-2026-11-05`, the endpoint as the owner measured
  it that day (tiny 1.0 s, 20 KiB 11.3 s, 103 KiB 14.8 s, 205 KiB 36.5 s),
  interpolated linearly between those points and extended past the last.
- The ~256 KiB body limit: a body over `--max-body-bytes` gets a 413, as the
  live endpoint answers.
- Overlap sensitivity (optional): with `--trip-above N`, a request that arrives
  while more than N are already in flight fails fast with a 502, the shape of
  the 2026-08-28 "four concurrent fail, four serial pass" result. Off by
  default; that result predates the endpoint's 2026-09-03 limit change and has
  not been re-measured.

Spans: it reports a `private_person` span on every occurrence of the
synthetic marker `Alice Example`, with codepoint offsets and the covered text,
so the witness's offset translation and span verification run for real.

It logs one line per request: body size, in-flight count, status, latency.
Never content. `GET /stats` returns totals as JSON.
"""

import argparse
import http.server
import json
import sys
import threading
import time

MARKER = "Alice Example"


class Stats:
    def __init__(self):
        self.lock = threading.Lock()
        self.in_flight = 0
        self.max_in_flight = 0
        self.requests = 0
        self.ok = 0
        self.tripped = 0
        self.too_large = 0
        self.body_bytes = 0
        self.busy_seconds = 0.0

    def snapshot(self):
        with self.lock:
            return {
                "requests": self.requests,
                "ok": self.ok,
                "tripped_502": self.tripped,
                "too_large_413": self.too_large,
                "max_in_flight": self.max_in_flight,
                "in_flight": self.in_flight,
                "body_bytes": self.body_bytes,
                "busy_seconds": round(self.busy_seconds, 3),
            }

    def reset(self):
        with self.lock:
            self.max_in_flight = self.in_flight
            self.requests = self.ok = self.tripped = self.too_large = 0
            self.body_bytes = 0
            self.busy_seconds = 0.0


STATS = Stats()
CONFIG = argparse.Namespace()


# (body KiB, seconds), measured 2026-11-05 against the live endpoint.
MEASURED_2026_11_05 = [(0.0, 1.0), (20.0, 11.3), (103.0, 14.8), (205.0, 36.5)]


def latency_seconds(body_bytes):
    if CONFIG.curve == "linear":
        return (CONFIG.base_ms + CONFIG.per_kib_ms * body_bytes / 1024.0) / 1000.0
    kib = body_bytes / 1024.0
    points = MEASURED_2026_11_05
    for (x0, y0), (x1, y1) in zip(points, points[1:]):
        if kib <= x1:
            return y0 + (y1 - y0) * (kib - x0) / (x1 - x0)
    (x0, y0), (x1, y1) = points[-2], points[-1]
    return y1 + (y1 - y0) * (kib - x1) / (x1 - x0)


def spans_for(text):
    spans = []
    search_from = 0
    while True:
        byte_index = text.find(MARKER, search_from)
        if byte_index < 0:
            break
        start = len(text[:byte_index])
        spans.append(
            {
                "category": "private_person",
                "start": start,
                "end": start + len(MARKER),
                "score": 0.99,
                "text": MARKER,
            }
        )
        search_from = byte_index + len(MARKER)
    return spans


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):
        pass

    def do_GET(self):
        if self.path == "/stats":
            return self.reply(200, STATS.snapshot())
        if self.path == "/stats/reset":
            STATS.reset()
            return self.reply(200, {"reset": True})
        return self.reply(404, {"error": "not found"})

    def do_POST(self):
        started = time.monotonic()
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        with STATS.lock:
            STATS.requests += 1
            STATS.body_bytes += length
            arrived_with = STATS.in_flight
            STATS.in_flight += 1
            STATS.max_in_flight = max(STATS.max_in_flight, STATS.in_flight)
        status = 200
        try:
            if not self.path.endswith("/privacy/classify"):
                status = 404
                return self.reply(404, {"error": "not found"})
            if length > CONFIG.max_body_bytes:
                status = 413
                with STATS.lock:
                    STATS.too_large += 1
                return self.reply(413, {"error": "Failed to buffer the request body"})
            if CONFIG.trip_above is not None and arrived_with >= CONFIG.trip_above:
                status = 502
                with STATS.lock:
                    STATS.tripped += 1
                time.sleep(0.2)
                return self.reply(502, {"error": "server_error"})
            request = json.loads(body)
            inputs = request["input"]
            if isinstance(inputs, str):
                inputs = [inputs]
            delay = latency_seconds(length)
            time.sleep(delay)
            with STATS.lock:
                STATS.ok += 1
            return self.reply(
                200,
                {
                    "data": [{"spans": spans_for(text)} for text in inputs],
                    "usage": {"input_tokens": sum(len(t) // 3 for t in inputs)},
                },
            )
        finally:
            elapsed = time.monotonic() - started
            with STATS.lock:
                STATS.in_flight -= 1
                STATS.busy_seconds += elapsed
            if CONFIG.verbose:
                print(
                    f"classify body_bytes={length} in_flight_on_arrival={arrived_with} "
                    f"status={status} seconds={elapsed:.3f}",
                    file=sys.stderr,
                    flush=True,
                )

    def reply(self, status, body):
        payload = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=8471)
    parser.add_argument("--base-ms", type=float, default=1300.0)
    parser.add_argument("--per-kib-ms", type=float, default=32.0)
    parser.add_argument("--max-body-bytes", type=int, default=256 * 1024)
    parser.add_argument("--curve", choices=["linear", "measured-2026-11-05"], default="linear")
    parser.add_argument("--trip-above", type=int, default=None)
    parser.add_argument("--verbose", action="store_true")
    parser.parse_args(namespace=CONFIG)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", CONFIG.port), Handler)
    server.daemon_threads = True
    print(f"stub classifier on 127.0.0.1:{CONFIG.port}", file=sys.stderr, flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
