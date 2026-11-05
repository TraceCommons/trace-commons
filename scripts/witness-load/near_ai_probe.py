#!/usr/bin/env python3
"""Measure NEAR AI's privacy/classify endpoint with synthetic text.

FOR THE KEY HOLDER TO RUN. Low volume, one request at a time, synthetic text
only, built here from a fixed vocabulary; it reads no file. It prints sizes,
statuses, entry counts and seconds -- never the key, never text, never a
response body (a failed response is reported by length and SHA-256).

The key is read from TRACE_NEAR_AI_PRIVACY_API_KEY. Use a key no driver or
witness is using while this runs: the endpoint has tripped on overlapping
traffic before, and a probe sharing a live key both disturbs that traffic
and reads its failures as its own (memory note on NEAR AI classify limits,
2026-08-28). Nothing is sent without --yes; without it the plan is printed.

  # the body-size limit: binary search with escape-heavy text, <= 8 requests
  near_ai_probe.py limit --yes

  # does an input array work under today's limit, and do its answers match
  # the same inputs sent one at a time?
  near_ai_probe.py batch --yes

  # and with the array's total near the body limit (e.g. 64 x 3 KiB)
  near_ai_probe.py batch --yes --batch-sizes 64 --batch-item-kib 3

  # per-request latency across body sizes, with a fitted base + per-KiB
  near_ai_probe.py latency --yes

  # how many requests may overlap: bursts of 2, then 4, then 8, a minute
  # apart, stopping at the first burst with any failure. This is the probe
  # that poisoned the key for minutes on 2026-08-28: run it last, on its own
  # key, and expect serial traffic on that key to fail for a while after a
  # failed burst.
  near_ai_probe.py overlap --yes --i-understand-overlap

Results decide two settings in trace-commons-protocol:
TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES (default 240 KiB) and
TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS (default 1, off).

If Python cannot verify TLS (python.org builds on macOS), set
SSL_CERT_FILE=/etc/ssl/cert.pem.
"""

import argparse
import hashlib
import threading
import json
import os
import random
import ssl
import sys
import time
import urllib.error
import urllib.request

DEFAULT_BASE_URL = "https://cloud-api.near.ai/v1"
DEFAULT_MODEL = "openai/privacy-filter"

WORDS = (
    "build compile link test cache module crate target debug release warning "
    "error function struct trait async future stream socket request response"
).split()


def escape_heavy_line(rng, index):
    """A line whose JSON encoding is much longer than its text: quotes,
    backslashes, tabs, control characters and multibyte characters."""
    tail = '\\"' * (index % 5)
    return (
        f'{index} "{rng.choice(WORDS)}" C:\\dir\\{rng.choice(WORDS)}\\f{index}\t'
        f"\u0001 caf\u00e9 \u4e2d\u6587 \u2192 {tail}\r\n"
    )


def body_for(model, text):
    return json.dumps({"model": model, "input": text}, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def text_with_body_size(model, rng, target_bytes):
    """Escape-heavy synthetic text whose request body is exactly
    `target_bytes`, or as close below it as one character allows."""
    lines = []
    index = 0
    while len(body_for(model, "".join(lines))) < target_bytes:
        lines.append(escape_heavy_line(rng, index))
        index += 1
    text = "".join(lines)
    # Trim one character at a time from the end until the body fits.
    while len(body_for(model, text)) > target_bytes:
        overshoot = len(body_for(model, text)) - target_bytes
        text = text[: -max(1, overshoot // 8)]
    return text


def small_text(rng, index):
    """A short synthetic text; every third carries a synthetic person and
    address, so there is something for the classifier to find."""
    if index % 3 == 0:
        return f"ask Alice Example (alice.example@example.com) about {rng.choice(WORDS)} {index}"
    return f"{rng.choice(WORDS)} {rng.choice(WORDS)} took {rng.randrange(1, 999)}ms, item {index}"


class Client:
    def __init__(self, args):
        self.url = args.base_url.rstrip("/") + "/privacy/classify"
        self.model = args.model
        self.sleep = args.sleep
        self.sent = 0
        self.max_requests = args.max_requests
        self.count_lock = threading.Lock()
        self.key = os.environ.get("TRACE_NEAR_AI_PRIVACY_API_KEY", "").strip()
        self.context = ssl.create_default_context(cafile=os.environ.get("SSL_CERT_FILE") or None)

    def post(self, body):
        if self.sent >= self.max_requests:
            raise SystemExit(f"stopping: --max-requests {self.max_requests} reached")
        if self.sent:
            time.sleep(self.sleep)
        return self.post_unpaced(body)

    def post_unpaced(self, body):
        """One request with no pause before it, for the overlap bursts. Still
        counted against --max-requests."""
        with self.count_lock:
            if self.sent >= self.max_requests:
                return "skipped:max-requests", None, 0.0
            self.sent += 1
        request = urllib.request.Request(
            self.url,
            data=body,
            method="POST",
            headers={"content-type": "application/json", "authorization": f"Bearer {self.key}"},
        )
        started = time.monotonic()
        try:
            with urllib.request.urlopen(request, timeout=120, context=self.context) as response:
                payload = response.read()
                status = response.status
        except urllib.error.HTTPError as error:
            payload = error.read()
            status = error.code
        except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
            return "transport:" + type(error).__name__, None, time.monotonic() - started
        seconds = time.monotonic() - started
        parsed = None
        if status == 200:
            try:
                parsed = json.loads(payload)
            except ValueError:
                parsed = None
        else:
            parsed = {"_len": len(payload), "_sha256": hashlib.sha256(payload).hexdigest()[:16]}
        return status, parsed, seconds


def report(**fields):
    print(" ".join(f"{key}={value}" for key, value in fields.items()), flush=True)


def spans_of(entry):
    return sorted(
        (span.get("category"), span.get("start"), span.get("end"))
        for span in (entry or {}).get("spans", [])
    )


def run_limit(client, args, rng):
    lo, hi = args.lo_kib * 1024, args.hi_kib * 1024
    report(mode="limit", lo_bytes=lo, hi_bytes=hi, note="200 below, 413 above")
    known_ok, known_too_large = None, None
    while hi - lo > args.resolution_kib * 1024:
        mid = (lo + hi) // 2
        body = body_for(client.model, text_with_body_size(client.model, rng, mid))
        status, parsed, seconds = client.post(body)
        extra = {}
        if isinstance(parsed, dict) and "_len" in parsed:
            extra = {"error_body_len": parsed["_len"], "error_body_sha256_16": parsed["_sha256"]}
        report(body_bytes=len(body), status=status, seconds=f"{seconds:.2f}", **extra)
        if status == 200:
            lo, known_ok = mid, len(body)
        elif status == 413:
            hi, known_too_large = mid, len(body)
        else:
            report(stop="status is neither 200 nor 413; not reading it as a size answer")
            break
    report(result="limit", largest_ok_body_bytes=known_ok, smallest_413_body_bytes=known_too_large)


def run_batch(client, args, rng):
    report(mode="batch", sizes=args.batch_sizes)
    for count in [int(value) for value in args.batch_sizes.split(",")]:
        texts = [small_text(rng, index) for index in range(count)]
        if args.batch_item_kib:
            # Pad each input to about this size, so a batch also tests the
            # body limit summed across inputs, not just the array shape.
            texts = [
                text + " " + text_with_body_size(client.model, rng, args.batch_item_kib * 1024)[:args.batch_item_kib * 700]
                for text in texts
            ]
        body = json.dumps({"model": client.model, "input": texts}, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        status, parsed, seconds = client.post(body)
        entries = parsed.get("data") if status == 200 and isinstance(parsed, dict) else None
        report(
            batch_inputs=count,
            body_bytes=len(body),
            status=status,
            seconds=f"{seconds:.2f}",
            entries=len(entries) if entries is not None else None,
            entries_match_inputs=(entries is not None and len(entries) == count),
            inputs_with_spans=(sum(1 for e in entries if e.get("spans")) if entries else None),
        )
        if not entries or len(entries) != count:
            continue
        # Do the batch's answers equal the same inputs sent alone? A few only.
        for index in range(min(args.compare, count)):
            single_status, single, single_seconds = client.post(body_for(client.model, texts[index]))
            single_entry = (single or {}).get("data", [None])[0] if single_status == 200 else None
            report(
                compare_index=index,
                single_status=single_status,
                single_seconds=f"{single_seconds:.2f}",
                same_spans=(single_entry is not None and spans_of(single_entry) == spans_of(entries[index])),
                span_count=len(spans_of(entries[index])),
            )


def run_latency(client, args, rng):
    report(mode="latency", sizes_kib=args.sizes_kib)
    points = []
    for size in [int(value) for value in args.sizes_kib.split(",")]:
        body = body_for(client.model, text_with_body_size(client.model, rng, size * 1024))
        status, parsed, seconds = client.post(body)
        usage = (parsed or {}).get("usage", {}) if status == 200 else {}
        report(body_bytes=len(body), body_kib=f"{len(body) / 1024:.1f}", status=status,
               seconds=f"{seconds:.2f}", input_tokens=usage.get("input_tokens"))
        if status == 200:
            points.append((len(body) / 1024.0, seconds))
    # Least-squares base + per-KiB, the shape the adapter's timeout takes
    # (TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS + _PER_KIB_MS). Compare it against
    # the defaults, 15 s + 0.25 s/KiB, with about 2x headroom.
    if len(points) >= 2:
        n = len(points)
        mean_x = sum(x for x, _ in points) / n
        mean_y = sum(y for _, y in points) / n
        sxx = sum((x - mean_x) ** 2 for x, _ in points)
        slope = sum((x - mean_x) * (y - mean_y) for x, y in points) / sxx if sxx else 0.0
        report(result="latency_fit", base_s=f"{mean_y - slope * mean_x:.2f}",
               per_kib_s=f"{slope:.4f}", points=n,
               worst_ratio_to_default_timeout=f"{max(y / (15 + 0.25 * x) for x, y in points):.2f}")


def run_overlap(client, args, rng):
    if not args.i_understand_overlap:
        raise SystemExit("overlap bursts can poison the key for minutes; add --i-understand-overlap")
    levels = [int(value) for value in args.overlap_levels.split(",")]
    report(mode="overlap", levels=args.overlap_levels, body_kib=args.overlap_body_kib,
           pause_s=args.overlap_pause)
    body = body_for(client.model, text_with_body_size(client.model, rng, args.overlap_body_kib * 1024))
    # Serial baseline first: a failure here means the key is already unwell
    # and nothing below would mean anything.
    status, _, seconds = client.post(body)
    report(phase="baseline", status=status, seconds=f"{seconds:.2f}")
    if status != 200:
        report(stop="baseline failed; not bursting")
        return
    for level in levels:
        time.sleep(args.overlap_pause)
        results = []
        lock = threading.Lock()
        start = threading.Barrier(level)

        def one():
            start.wait()
            outcome = client.post_unpaced(body)
            with lock:
                results.append(outcome)

        threads = [threading.Thread(target=one) for _ in range(level)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        statuses = {}
        for status, _, _ in results:
            statuses[str(status)] = statuses.get(str(status), 0) + 1
        slowest = max(seconds for _, _, seconds in results)
        report(phase="burst", in_flight=level, statuses=json.dumps(statuses, separators=(",", ":")),
               slowest_s=f"{slowest:.2f}")
        # Serial follow-up after the same pause: did the burst poison the key?
        time.sleep(args.overlap_pause)
        status, _, seconds = client.post(body)
        report(phase="after_burst", in_flight=level, status=status, seconds=f"{seconds:.2f}")
        if set(statuses) != {"200"} or status != 200:
            report(stop=f"failures at {level} in flight; not escalating")
            break


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("mode", choices=["limit", "batch", "latency", "overlap"])
    parser.add_argument("--yes", action="store_true", help="actually send requests")
    parser.add_argument("--base-url", default=os.environ.get("TRACE_NEAR_AI_PRIVACY_BASE_URL", DEFAULT_BASE_URL))
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--sleep", type=float, default=5.0, help="seconds between requests")
    parser.add_argument("--max-requests", type=int, default=12)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--lo-kib", type=int, default=200)
    parser.add_argument("--hi-kib", type=int, default=320)
    parser.add_argument("--resolution-kib", type=int, default=4)
    parser.add_argument("--batch-sizes", default="4,16,64")
    parser.add_argument("--compare", type=int, default=2, help="batch inputs re-sent alone to compare")
    parser.add_argument("--batch-item-kib", type=int, default=0,
                        help="pad each batch input to about this size (0: short texts)")
    parser.add_argument("--sizes-kib", default="1,4,16,32,64,96,128,160,200")
    parser.add_argument("--overlap-levels", default="2,4,8")
    parser.add_argument("--overlap-body-kib", type=int, default=8)
    parser.add_argument("--overlap-pause", type=float, default=60.0,
                        help="seconds before each burst and before each follow-up")
    parser.add_argument("--i-understand-overlap", action="store_true")
    args = parser.parse_args()
    loopback = args.base_url.startswith(("http://127.0.0.1", "http://localhost"))
    if not (args.base_url.startswith("https://") or loopback):
        raise SystemExit("refusing a non-https base URL: the key rides on every request")
    client = Client(args)
    if not args.yes:
        report(plan=args.mode, url=client.url, model=client.model, sleep_s=args.sleep,
               max_requests=args.max_requests, key_present=bool(client.key),
               note="nothing sent; add --yes")
        return
    if not client.key:
        raise SystemExit("TRACE_NEAR_AI_PRIVACY_API_KEY is not set")
    rng = random.Random(args.seed)
    if args.mode == "overlap":
        # 1 baseline + per level (burst + follow-up); the default 12 is too
        # few for 2,4,8, so size the cap to the plan and no larger.
        client.max_requests = 1 + sum(int(v) + 1 for v in args.overlap_levels.split(","))
    {"limit": run_limit, "batch": run_batch, "latency": run_latency,
     "overlap": run_overlap}[args.mode](client, args, rng)
    report(requests_sent=client.sent)


if __name__ == "__main__":
    main()
