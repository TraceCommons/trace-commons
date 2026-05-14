#!/usr/bin/env python3
"""Summarize A2.6 bake-off progress from a structured tracing log file.

The bake-off binary writes structured tracing events to a log on the Lambda
H100 host. This helper parses those events and produces a short operator-side
status summary: which candidates have finished, which one is currently being
scored, how long the run has been going, and a rough ETA extrapolated from
the candidates that have already completed.

Usage:

    python3 scripts/operator/bakeoff-progress.py \
        --log ~/bakeoff/bakeoff-a26.log

    python3 scripts/operator/bakeoff-progress.py --watch
    python3 scripts/operator/bakeoff-progress.py --out json

Hash-only audit note: this script reads a local operator log and prints a
short summary to stdout. It must not be piped to a remote sink that would
exfiltrate operator-secret material; nothing in the parsed events should
contain contributor identity or envelope content, but operators are
responsible for confirming that before forwarding.

Pure-stdlib (argparse + re + datetime + json + time).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Optional


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------

# Tracing default format: "2026-05-14T11:11:37.001234Z  LEVEL target: message"
_TS_RE = re.compile(r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z)\b")

# Event keyword set we care about (everything else, including truncation
# WARNs, is ignored).
_EVENT_PATTERNS = {
    "bakeoff_start": re.compile(r"\bbakeoff_start\b"),
    "load_start": re.compile(r"\bbakeoff_candidate_load_start\b"),
    "load_done": re.compile(r"\bbakeoff_candidate_load_done\b"),
    "eval_start": re.compile(r"\bcandidate_eval_start\b"),
    "eval_complete": re.compile(r"\brun_candidate_eval complete\b"),
    "candidate_done": re.compile(r"\bbakeoff_candidate_done\b"),
    "bakeoff_done": re.compile(r"\bbakeoff_done\b"),
}

_KV_RE = re.compile(r"(\w+)=([^\s]+)")


def _parse_ts(s: str) -> datetime:
    # datetime.fromisoformat accepts the form once we strip the trailing 'Z'
    # and reattach +00:00.
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    return datetime.fromisoformat(s)


def _parse_kv(line: str) -> dict:
    out = {}
    for k, v in _KV_RE.findall(line):
        out[k] = v
    return out


def _classify(line: str) -> Optional[str]:
    for name, pat in _EVENT_PATTERNS.items():
        if pat.search(line):
            return name
    return None


def parse_log(text: str) -> dict:
    """Walk a bake-off log and produce a structured summary dict.

    The returned dict has:
        run_start:        datetime | None
        candidate_count:  int | None
        candidates:       ordered dict {id -> candidate state}
        last_event_ts:    datetime | None  (latest parsed event timestamp)
        bakeoff_done:     bool
    """

    run_start: Optional[datetime] = None
    candidate_count: Optional[int] = None
    bakeoff_done = False
    last_event_ts: Optional[datetime] = None

    # We preserve insertion order so the report follows the run order.
    candidates: dict = {}

    def _ensure(cid: str) -> dict:
        if cid not in candidates:
            candidates[cid] = {
                "id": cid,
                "status": "pending",
                "load_started_at": None,
                "load_done_at": None,
                "eval_started_at": None,
                "eval_done_at": None,
                "load_elapsed_seconds": None,
                "score_elapsed_seconds": None,
                "auc": None,
                "para_delta": None,
                "throughput_tps": None,
                "peak_vram": None,
            }
        return candidates[cid]

    for raw in text.splitlines():
        line = raw.rstrip()
        if not line:
            continue
        m = _TS_RE.match(line)
        if not m:
            continue
        try:
            ts = _parse_ts(m.group("ts"))
        except ValueError:
            continue
        kind = _classify(line)
        if kind is None:
            continue
        last_event_ts = ts
        kv = _parse_kv(line)

        if kind == "bakeoff_start":
            run_start = ts
            cc = kv.get("candidate_count")
            if cc is not None:
                try:
                    candidate_count = int(cc)
                except ValueError:
                    pass
            continue

        if kind == "bakeoff_done":
            bakeoff_done = True
            continue

        cid = kv.get("candidate_id")
        if cid is None:
            continue
        c = _ensure(cid)

        if kind == "load_start":
            c["status"] = "loading"
            c["load_started_at"] = ts
        elif kind == "load_done":
            c["status"] = "loaded"
            c["load_done_at"] = ts
            v = kv.get("load_elapsed_seconds")
            if v is not None:
                try:
                    c["load_elapsed_seconds"] = float(v)
                except ValueError:
                    pass
        elif kind == "eval_start":
            c["status"] = "scoring"
            c["eval_started_at"] = ts
        elif kind == "eval_complete":
            # We mostly read final stats from candidate_done, but keep AUC
            # here as a fallback in case the run was killed between
            # eval_complete and candidate_done.
            for k_src, k_dst, conv in (
                ("auc", "auc", float),
                ("para_delta", "para_delta", float),
                ("tokens_per_second", "throughput_tps", float),
                ("peak_vram", "peak_vram", int),
            ):
                v = kv.get(k_src)
                if v is None:
                    continue
                try:
                    c[k_dst] = conv(v)
                except ValueError:
                    pass
        elif kind == "candidate_done":
            c["status"] = "done"
            c["eval_done_at"] = ts
            for k_src, k_dst, conv in (
                ("score_elapsed_seconds", "score_elapsed_seconds", float),
                ("auc", "auc", float),
                ("throughput_tps", "throughput_tps", float),
            ):
                v = kv.get(k_src)
                if v is None:
                    continue
                try:
                    c[k_dst] = conv(v)
                except ValueError:
                    pass

    return {
        "run_start": run_start,
        "candidate_count": candidate_count,
        "candidates": candidates,
        "last_event_ts": last_event_ts,
        "bakeoff_done": bakeoff_done,
    }


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


def _fmt_duration(seconds: float) -> str:
    seconds = max(0.0, float(seconds))
    h = int(seconds // 3600)
    m = int((seconds % 3600) // 60)
    if h > 0:
        return f"{h}h {m:02d}m"
    if m > 0:
        return f"{m}m"
    return f"{int(seconds)}s"


def _fmt_minutes(seconds: float) -> str:
    return f"{seconds / 60.0:.1f}m"


def _fmt_vram(bytes_: Optional[int]) -> Optional[str]:
    if bytes_ is None:
        return None
    return f"{bytes_ / (1024 ** 3):.1f}GB"


def _completed_durations(candidates: dict) -> list:
    out = []
    for c in candidates.values():
        if c["status"] != "done":
            continue
        load = c["load_elapsed_seconds"] or 0.0
        score = c["score_elapsed_seconds"] or 0.0
        if load + score > 0:
            out.append(load + score)
    return out


def _current_phase_elapsed(c: dict, now: datetime) -> Optional[float]:
    if c["status"] == "loading" and c["load_started_at"]:
        return (now - c["load_started_at"]).total_seconds()
    if c["status"] in ("loaded", "scoring") and c["eval_started_at"]:
        return (now - c["eval_started_at"]).total_seconds()
    if c["status"] == "loaded" and c["load_done_at"]:
        return (now - c["load_done_at"]).total_seconds()
    return None


def render_text(summary: dict, now: datetime) -> str:
    run_start = summary["run_start"]
    candidates = summary["candidates"]
    candidate_count = summary["candidate_count"] or len(candidates)

    lines = ["A2.6 BAKE-OFF SUMMARY"]
    if run_start is None:
        lines.append("(no bakeoff_start event observed in log)")
        return "\n".join(lines) + "\n"

    elapsed = (now - run_start).total_seconds()
    completed = sum(1 for c in candidates.values() if c["status"] == "done")
    lines.append(f"Started: {run_start.strftime('%Y-%m-%d %H:%M:%S')} UTC")
    lines.append(f"Elapsed: {_fmt_duration(elapsed)}")
    lines.append(f"Candidates: {completed} of {candidate_count} complete")
    lines.append("")

    # Determine a label column width for alignment.
    label_width = max((len(cid) for cid in candidates), default=0) + 1
    label_width = max(label_width, 24)

    for c in candidates.values():
        cid = c["id"]
        label = f"{cid}:".ljust(label_width)
        if c["status"] == "done":
            auc = c["auc"]
            pd = c["para_delta"]
            tps = c["throughput_tps"]
            vram = _fmt_vram(c["peak_vram"])
            scored = c["score_elapsed_seconds"]
            parts = ["DONE"]
            if auc is not None:
                parts.append(f"AUC={auc:.3f}")
            if pd is not None:
                parts.append(f"para_delta={pd:.3f}")
            if tps is not None:
                parts.append(f"throughput={tps:.0f}tps")
            if vram is not None:
                parts.append(f"vram={vram}")
            if scored is not None:
                parts.append(f"(scored in {_fmt_minutes(scored)})")
            lines.append(f"{label}{'  '.join(parts)}")
        elif c["status"] in ("loading", "loaded", "scoring"):
            phase = "LOADING" if c["status"] == "loading" else "SCORING"
            phase_elapsed = _current_phase_elapsed(c, now)
            if phase_elapsed is not None:
                lines.append(f"{label}{phase}  elapsed {_fmt_minutes(phase_elapsed)}")
            else:
                lines.append(f"{label}{phase}")
        else:
            lines.append(f"{label}PENDING")

    # Render placeholder rows for candidates the run will load later but
    # whose IDs we haven't seen yet (i.e. candidate_count > observed).
    unobserved = candidate_count - len(candidates)
    if unobserved > 0:
        for i in range(unobserved):
            slot = f"(pending slot {len(candidates) + i + 1}):".ljust(label_width)
            lines.append(f"{slot}PENDING")

    # ETA: only if at least one candidate is done and we know candidate_count.
    durations = _completed_durations(candidates)
    pending = candidate_count - completed
    if durations and pending > 0:
        avg = sum(durations) / len(durations)
        # Subtract any time already spent on the in-flight candidate (its
        # phase elapsed counts toward its share of avg).
        in_flight_elapsed = 0.0
        for c in candidates.values():
            if c["status"] in ("loading", "loaded", "scoring"):
                e = _current_phase_elapsed(c, now)
                if e is not None:
                    in_flight_elapsed = max(in_flight_elapsed, e)
        remaining = max(0.0, avg * pending - in_flight_elapsed)
        eta_dt = now + timedelta(seconds=remaining)
        lines.append("")
        lines.append(
            f"ETA: ~{eta_dt.strftime('%H:%M')} UTC "
            f"(extrapolated from {len(durations)} completed candidate"
            f"{'s' if len(durations) != 1 else ''})"
        )
    elif summary["bakeoff_done"]:
        lines.append("")
        lines.append("bakeoff_done observed; run complete.")

    return "\n".join(lines) + "\n"


def render_json(summary: dict, now: datetime) -> str:
    def _iso(dt: Optional[datetime]) -> Optional[str]:
        return None if dt is None else dt.isoformat().replace("+00:00", "Z")

    candidates_out = []
    for c in summary["candidates"].values():
        candidates_out.append({
            "id": c["id"],
            "status": c["status"],
            "load_elapsed_seconds": c["load_elapsed_seconds"],
            "score_elapsed_seconds": c["score_elapsed_seconds"],
            "auc": c["auc"],
            "para_delta": c["para_delta"],
            "throughput_tps": c["throughput_tps"],
            "peak_vram": c["peak_vram"],
            "load_started_at": _iso(c["load_started_at"]),
            "eval_started_at": _iso(c["eval_started_at"]),
            "eval_done_at": _iso(c["eval_done_at"]),
            "current_phase_elapsed_seconds": _current_phase_elapsed(c, now),
        })

    run_start = summary["run_start"]
    completed = sum(1 for c in summary["candidates"].values() if c["status"] == "done")
    out = {
        "run_start": _iso(run_start),
        "now": _iso(now),
        "elapsed_seconds": None if run_start is None else (now - run_start).total_seconds(),
        "candidate_count": summary["candidate_count"],
        "completed": completed,
        "bakeoff_done": summary["bakeoff_done"],
        "candidates": candidates_out,
    }
    return json.dumps(out, indent=2, sort_keys=False) + "\n"


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _read_log(path: Path) -> str:
    with path.open("r", encoding="utf-8", errors="replace") as fh:
        return fh.read()


def _resolve_now(arg: Optional[str], summary: dict) -> datetime:
    if arg is not None:
        return _parse_ts(arg)
    # If summary has a last_event_ts and no explicit "now", we still want
    # wall-clock now for live runs. For deterministic tests, callers pass
    # --now explicitly.
    return datetime.now(timezone.utc)


def main(argv: Optional[list] = None) -> int:
    p = argparse.ArgumentParser(description="Summarize A2.6 bake-off progress.")
    p.add_argument(
        "--log",
        default="~/bakeoff/bakeoff-a26.log",
        help="Path to the bake-off tracing log (default: ~/bakeoff/bakeoff-a26.log).",
    )
    p.add_argument(
        "--watch",
        action="store_true",
        help="Re-emit the summary every 30 seconds (Ctrl-C to exit).",
    )
    p.add_argument(
        "--out",
        choices=("text", "json"),
        default="text",
        help="Output format (default: text).",
    )
    p.add_argument(
        "--now",
        default=None,
        help="Override wall-clock 'now' with an ISO8601 UTC timestamp "
             "(used by tests; not normally needed).",
    )
    args = p.parse_args(argv)

    log_path = Path(os.path.expanduser(args.log))
    if not log_path.exists():
        print(f"bakeoff-progress: log not found: {log_path}", file=sys.stderr)
        return 2

    def _emit() -> None:
        summary = parse_log(_read_log(log_path))
        now = _resolve_now(args.now, summary)
        if args.out == "json":
            sys.stdout.write(render_json(summary, now))
        else:
            sys.stdout.write(render_text(summary, now))
        sys.stdout.flush()

    if not args.watch:
        _emit()
        return 0

    try:
        while True:
            _emit()
            sys.stdout.write("\n")
            sys.stdout.flush()
            time.sleep(30)
    except KeyboardInterrupt:
        return 0


if __name__ == "__main__":
    sys.exit(main())
