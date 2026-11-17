#!/usr/bin/env python3
"""Load-test a redaction witness with synthetic contributions.

SYNTHETIC ONLY. Every request body is generated here from a fixed vocabulary
(log lines, paths under /home/synthetic, a marker name `Alice Example`); no
file is read and no real transcript can reach a witness through this script.
The default target is a local witness on 127.0.0.1. Pointing it at a deployed
witness is allowed only at low volume (a handful of requests), and never at a
witness whose classifier key is shared with a live driver: concurrent requests
there spend the same NEAR AI key and have tripped it before.

Usage:

  # one 241 KB tool output, once
  witness_load.py run --size-kb 241

  # 8 workers, 32 requests, sizes drawn from a list
  witness_load.py run --concurrency 8 --requests 32 --size-kb 50,250,900

  # many events instead of one large one (closer to a real session's shape)
  witness_load.py run --size-kb 241 --events 120

  # write one body to a file instead of sending it
  witness_load.py body --size-kb 241 --events 120 > body.json

Reports per-request latency percentiles (p50/p90/p99/max), throughput in
requests/s and synthetic KB/s, and a count per HTTP status. A 503 is the
witness's "busy" answer and is counted, not retried.
"""

import argparse
import hashlib
import json
import os
import random
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid

WORDS = (
    "build compile link test pass fail retry cache module crate target debug "
    "release warning error note help function struct impl trait async await "
    "future stream buffer socket request response header body status latency "
    "window token span offset codepoint placeholder envelope certificate"
).split()


def synthetic_line(rng, index):
    kind = rng.randrange(6)
    if kind == 0:
        return f"[{index:06d}] INFO  {rng.choice(WORDS)}::{rng.choice(WORDS)} took {rng.randrange(1, 9999)}ms"
    if kind == 1:
        return f"/home/synthetic/project/src/{rng.choice(WORDS)}/{rng.choice(WORDS)}.rs:{rng.randrange(1, 4000)}:{rng.randrange(1, 120)}"
    if kind == 2:
        return f"warning: unused variable `{rng.choice(WORDS)}_{rng.randrange(100)}` in {rng.choice(WORDS)}"
    if kind == 3:
        return "    " + " ".join(rng.choice(WORDS) for _ in range(rng.randrange(4, 14)))
    if kind == 4:
        return f"{rng.randrange(16**12):012x} {rng.choice(WORDS)} {rng.randrange(10**6)} {rng.choice(WORDS)}"
    if rng.randrange(20) == 0:
        return f"commit by Alice Example on {rng.choice(WORDS)}"
    return f"test {rng.choice(WORDS)}_{rng.choice(WORDS)} ... ok"


def synthetic_text(rng, size_bytes):
    lines = []
    total = 0
    index = 0
    while total < size_bytes:
        line = synthetic_line(rng, index)
        lines.append(line)
        total += len(line) + 1
        index += 1
    return "\n".join(lines)[:size_bytes]


def timestamp(offset_ms):
    base = time.time() + offset_ms / 1000.0
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(base)) + ".000000Z"


def event(**fields):
    """An event with every field the template contribution carries, so the
    witness's strict parse accepts it."""
    base = {
        "content": None,
        "cost_usd": None,
        "latency_ms": None,
        "structured_payload": None,
        "token_counts": None,
        "tool_name": None,
    }
    base.update(fields)
    return base


def synthetic_body(rng, size_bytes, events):
    """One witness request: a raw contribution whose tool output totals
    `size_bytes`, split across `events` tool calls and results."""
    events = max(1, events)
    per_event = max(1, size_bytes // events)
    body_events = [
        event(**{
            "content": "run the synthetic check",
            "event_id": str(uuid.UUID(int=rng.getrandbits(128), version=4)),
            "event_type": "user_message",
            "structured_payload": {"state": "Completed"},
            "timestamp": timestamp(0),
        })
    ]
    for index in range(events):
        call_id = f"call_{index}"
        call_event = str(uuid.UUID(int=rng.getrandbits(128), version=4))
        body_events.append(
            event(**{
                "event_id": call_event,
                "event_type": "tool_call",
                "structured_payload": {"arguments": {"cmd": f"echo synthetic {index}"}},
                "success": True,
                "timestamp": timestamp(index * 10 + 1),
                "tool_call_id": call_id,
                "tool_name": "shell",
            })
        )
        body_events.append(
            event(**{
                "content": synthetic_text(rng, per_event),
                "event_id": str(uuid.UUID(int=rng.getrandbits(128), version=4)),
                "event_type": "tool_result",
                "parent_event_id": call_event,
                "success": True,
                "timestamp": timestamp(index * 10 + 2),
                "tool_call_id": call_id,
                "tool_name": "shell",
            })
        )
    body_events.append(
        event(**{
            "content": "done",
            "event_id": str(uuid.UUID(int=rng.getrandbits(128), version=4)),
            "event_type": "assistant_message",
            "latency_ms": 10,
            "timestamp": timestamp(events * 10 + 5),
        })
    )
    return {
        "granted_scopes": ["debugging_evaluation"],
        "granted_uses": ["debugging"],
        "raw_contribution": {
            "consent": {
                "correction_included": False,
                "message_text_included": True,
                "policy_version": "2026-04-24",
                "revocable": True,
                "routing_metadata_included": False,
                "scopes": ["debugging_evaluation"],
                "tool_payloads_included": True,
            },
            "contributor": {"revocation_handle": str(uuid.UUID(int=rng.getrandbits(128), version=4))},
            "created_at": timestamp(0),
            "embedding_analysis": None,
            "events": body_events,
            "ironclaw": {"channel": "cli", "version": "0.1.0"},
            "outcome": {"task_success": "success", "user_feedback": "none"},
            "replay": {
                "replay_notes": ["Synthetic load-test contribution."],
                "replayable": True,
                "required_tools": ["shell"],
            },
            "submission_id": str(uuid.UUID(int=rng.getrandbits(128), version=4)),
            "trace_id": str(uuid.UUID(int=rng.getrandbits(128), version=4)),
            "value": {"credit_points_pending": 0.0, "submission_score": 0.0},
        },
    }


def percentile(sorted_values, fraction):
    if not sorted_values:
        return float("nan")
    index = min(len(sorted_values) - 1, max(0, int(round(fraction * (len(sorted_values) - 1)))))
    return sorted_values[index]


def send(url, payload, timeout):
    """Submit one job and poll it to its answer.

    `url` is the jobs route. The latency is submit to final answer, which is
    what a contributor waits. A refusal at submit (busy, malformed) is
    reported with its status, as before.
    """
    secret = os.urandom(32)
    started = time.monotonic()
    deadline = started + timeout
    submit = urllib.request.Request(
        url,
        data=payload,
        method="POST",
        headers={
            "content-type": "application/json",
            "x-trace-witness-fetch-secret-sha256": hashlib.sha256(secret).hexdigest(),
        },
    )
    try:
        with urllib.request.urlopen(submit, timeout=timeout) as response:
            accepted = json.loads(response.read())
            if response.status != 202:
                return response.status, time.monotonic() - started
    except urllib.error.HTTPError as error:
        error.read()
        return error.code, time.monotonic() - started
    except (urllib.error.URLError, TimeoutError, ConnectionError):
        return "transport", time.monotonic() - started
    job_url = url.rstrip("/") + "/" + accepted["job_id"]
    wait = accepted.get("poll_after_secs", 1)
    while True:
        if time.monotonic() + wait > deadline:
            return "timeout", time.monotonic() - started
        time.sleep(max(1, min(wait, 60)))
        poll = urllib.request.Request(
            job_url, method="GET", headers={"x-trace-witness-fetch-secret": secret.hex()}
        )
        try:
            with urllib.request.urlopen(poll, timeout=30) as response:
                body = response.read()
                if response.status == 202:
                    wait = json.loads(body).get("poll_after_secs", 1)
                    continue
                return response.status, time.monotonic() - started
        except urllib.error.HTTPError as error:
            error.read()
            return error.code, time.monotonic() - started
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            return "transport", time.monotonic() - started


def run(args):
    sizes_kb = [float(value) for value in args.size_kb.split(",")]
    rng = random.Random(args.seed)
    plan = []
    for index in range(args.requests):
        size = int(sizes_kb[index % len(sizes_kb)] * 1000)
        plan.append((size, json.dumps(synthetic_body(rng, size, args.events)).encode("utf-8")))
    url = args.url.rstrip("/") + args.path
    results = []
    results_lock = threading.Lock()
    queue = list(enumerate(plan))
    queue_lock = threading.Lock()

    def worker():
        while True:
            with queue_lock:
                if not queue:
                    return
                index, (size, payload) = queue.pop(0)
            status, seconds = send(url, payload, args.timeout)
            with results_lock:
                results.append((index, size, status, seconds))
            if args.verbose:
                print(f"request={index} size_kb={size / 1000:.0f} status={status} seconds={seconds:.2f}",
                      file=sys.stderr, flush=True)

    started = time.monotonic()
    threads = [threading.Thread(target=worker) for _ in range(max(1, args.concurrency))]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    wall = time.monotonic() - started

    ok = sorted(seconds for _, _, status, seconds in results if status == 200)
    by_status = {}
    for _, _, status, _ in results:
        by_status[str(status)] = by_status.get(str(status), 0) + 1
    ok_kb = sum(size for _, size, status, _ in results if status == 200) / 1000.0
    report = {
        "url": url,
        "requests": len(results),
        "concurrency": args.concurrency,
        "events_per_request": args.events,
        "size_kb": sizes_kb,
        "status_counts": by_status,
        "wall_seconds": round(wall, 2),
        "ok_latency_seconds": {
            "p50": round(percentile(ok, 0.50), 2),
            "p90": round(percentile(ok, 0.90), 2),
            "p99": round(percentile(ok, 0.99), 2),
            "max": round(ok[-1], 2) if ok else None,
        },
        "throughput_requests_per_s": round(len(ok) / wall, 3) if wall else None,
        "throughput_kb_per_s": round(ok_kb / wall, 1) if wall else None,
    }
    print(json.dumps(report, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("run", "body"):
        command = sub.add_parser(name)
        command.add_argument("--size-kb", default="241",
                             help="synthetic tool-output size per request, comma list cycles")
        command.add_argument("--events", type=int, default=1,
                             help="split each request's tool output across this many tool results")
        command.add_argument("--seed", type=int, default=1)
    run_command = sub.choices["run"]
    run_command.add_argument("--url", default="http://127.0.0.1:8088")
    run_command.add_argument("--path", default="/v1/witness/jobs")
    run_command.add_argument("--concurrency", type=int, default=1)
    run_command.add_argument("--requests", type=int, default=1)
    run_command.add_argument("--timeout", type=float, default=600.0)
    run_command.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    if args.command == "body":
        rng = random.Random(args.seed)
        size = int(float(args.size_kb.split(",")[0]) * 1000)
        json.dump(synthetic_body(rng, size, args.events), sys.stdout)
        return
    run(args)


if __name__ == "__main__":
    main()
