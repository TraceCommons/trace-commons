#!/usr/bin/env python3
"""Witness overload monitor: polls the redaction witness's /v1/diagnostics.

Stdlib only. Label-only: records, state and stdout carry counts and fixed
labels, never the witness URL, its measurement or signing address, or any
exception text (an exception string can embed the URL).

  witness_monitor.py poll    one poll for a systemd timer: appends a record,
                             exits 1 when any alert is raised
  witness_monitor.py check   stateless deploy smoke: prints
                             WitnessHealthOK / WitnessHealthFailure:<labels>

Exit codes: 0 healthy, 1 alert, 2 configuration refused.
Runbook: docs/operator/witness-monitor.md
"""
import datetime
import json
import os
import pathlib
import sys
import urllib.error
import urllib.request

SCHEMA = "witness-diagnostics-v1"
PREFIX = "TRACE_COMMONS_WITNESS_MONITOR_"
REQUEST_FIELDS = (
    "max_concurrent",
    "in_use",
    "completed_2xx",
    "completed_4xx",
    "completed_5xx",
    "saturated_refusals",
    "timed_out",
)
MAX_BODY_BYTES = 1 << 20


class ConfigError(Exception):
    """The message is a fixed label, never the offending value."""


class Config:
    def __init__(
        self,
        url,
        timeout_secs,
        saturated_delta,
        timed_out_delta,
        full_polls,
        records_path,
        state_path,
        max_records_bytes,
    ):
        self.url = url
        self.timeout_secs = timeout_secs
        self.saturated_delta = saturated_delta
        self.timed_out_delta = timed_out_delta
        self.full_polls = full_polls
        self.records_path = records_path
        self.state_path = state_path
        self.max_records_bytes = max_records_bytes


def _int(env, name, default, minimum=1):
    raw = env.get(PREFIX + name)
    if raw is None or raw == "":
        return default
    try:
        value = int(raw)
    except ValueError:
        raise ConfigError("config_invalid:" + name.lower()) from None
    if value < minimum:
        raise ConfigError("config_invalid:" + name.lower())
    return value


def config_from_env(env):
    url = env.get(PREFIX + "URL", "")
    if not url:
        raise ConfigError("config_missing:url")
    if not (url.startswith("https://") or url.startswith("http://")):
        raise ConfigError("config_invalid:url")
    state_dir = pathlib.Path(
        env.get(PREFIX + "STATE_DIR", "/var/lib/tracecommons/witness-monitor")
    )
    return Config(
        url=url,
        timeout_secs=float(_int(env, "TIMEOUT_SECS", 10)),
        saturated_delta=_int(env, "SATURATED_DELTA", 1),
        timed_out_delta=_int(env, "TIMED_OUT_DELTA", 1),
        full_polls=_int(env, "FULL_POLLS", 3),
        records_path=state_dir / "records.jsonl",
        state_path=state_dir / "state.json",
        max_records_bytes=_int(env, "MAX_RECORDS_BYTES", 10 * 1024 * 1024),
    )


def fetch(url, timeout_secs):
    """Returns (failure_label, body). The label never carries the URL."""
    try:
        with urllib.request.urlopen(url, timeout=timeout_secs) as response:
            raw = response.read(MAX_BODY_BYTES + 1)
    except urllib.error.HTTPError as err:
        return ("http_5xx" if err.code >= 500 else "http_4xx"), None
    except urllib.error.URLError as err:
        if isinstance(err.reason, TimeoutError):
            return "timeout", None
        return "connection", None
    except TimeoutError:
        return "timeout", None
    except Exception:
        return "connection", None
    if len(raw) > MAX_BODY_BYTES:
        return "body_not_json", None
    try:
        return None, json.loads(raw)
    except ValueError:
        return "body_not_json", None


def _is_count(value):
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def validate(body):
    """None when the body has the shape this monitor reads, else a label."""
    if not isinstance(body, dict) or body.get("schema") != SCHEMA:
        return "schema_mismatch"
    requests = body.get("requests")
    if not isinstance(requests, dict):
        return "schema_mismatch"
    for field in REQUEST_FIELDS:
        if not _is_count(requests.get(field)):
            return "schema_mismatch"
    if requests["max_concurrent"] < 1 or not _is_count(body.get("uptime_secs")):
        return "schema_mismatch"
    return None


def evaluate(body, prev, cfg):
    """Returns (alerts, new_state, record_fields). `body` is validated."""
    req = body["requests"]
    uptime = body["uptime_secs"]
    restarted = bool(
        prev
        and (
            uptime < prev["uptime_secs"]
            or req["saturated_refusals"] < prev["saturated_refusals"]
            or req["timed_out"] < prev["timed_out"]
        )
    )
    baseline = prev if prev and not restarted else None
    sat_delta = req["saturated_refusals"] - baseline["saturated_refusals"] if baseline else 0
    to_delta = req["timed_out"] - baseline["timed_out"] if baseline else 0
    full = req["in_use"] >= req["max_concurrent"]
    streak = (baseline["full_streak"] + 1 if baseline else 1) if full else 0

    alerts = []
    if sat_delta >= cfg.saturated_delta:
        alerts.append("saturated_refusals_rose")
    if to_delta >= cfg.timed_out_delta:
        alerts.append("timed_out_rose")
    if streak >= cfg.full_polls:
        alerts.append("slots_full_sustained")

    state = {
        "uptime_secs": uptime,
        "saturated_refusals": req["saturated_refusals"],
        "timed_out": req["timed_out"],
        "full_streak": streak,
    }
    record = {
        "in_use": req["in_use"],
        "max_concurrent": req["max_concurrent"],
        "saturated_refusals": req["saturated_refusals"],
        "timed_out": req["timed_out"],
        "saturated_delta": sat_delta,
        "timed_out_delta": to_delta,
        "full_streak": streak,
        "uptime_secs": uptime,
        "restarted": restarted,
    }
    return alerts, state, record


def _load_state(path):
    try:
        state = json.loads(pathlib.Path(path).read_text())
        for key in ("uptime_secs", "saturated_refusals", "timed_out", "full_streak"):
            if not _is_count(state.get(key)):
                return None
        return state
    except (OSError, ValueError, AttributeError):
        return None


def _write_state(path, state):
    path = pathlib.Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(state, sort_keys=True))
    os.replace(tmp, path)


def _append_record(cfg, record):
    path = pathlib.Path(cfg.records_path)
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        if path.stat().st_size >= cfg.max_records_bytes:
            os.replace(path, path.with_name(path.name + ".1"))
    except FileNotFoundError:
        pass
    with open(path, "a") as handle:
        handle.write(json.dumps(record, sort_keys=True) + "\n")


def _now():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def poll(cfg, fetcher=fetch, now=None):
    now = now or _now()
    failure, body = fetcher(cfg.url, cfg.timeout_secs)
    record = {"ts": now}
    alerts = []
    if failure:
        # Fail closed: an unreachable witness is an alert, and the baseline
        # is kept so deltas still mean something when it answers again.
        alerts = ["unreachable"]
        record["failure"] = failure
    else:
        bad = validate(body)
        if bad:
            alerts = [bad]
        else:
            alerts, state, fields = evaluate(
                body, _load_state(cfg.state_path), cfg
            )
            record.update(fields)
            _write_state(cfg.state_path, state)
    record["alerts"] = alerts
    record["status"] = "alert" if alerts else "ok"
    _append_record(cfg, record)
    print("WitnessMonitor %s%s" % (record["status"], "".join(" " + a for a in alerts)))
    return 1 if alerts else 0


def check(cfg, fetcher=fetch):
    failure, body = fetcher(cfg.url, cfg.timeout_secs)
    if failure:
        labels = ["unreachable:" + failure]
    else:
        bad = validate(body)
        if bad:
            labels = [bad]
        elif body["requests"]["in_use"] >= body["requests"]["max_concurrent"]:
            labels = ["slots_full"]
        else:
            labels = []
    if labels:
        print("WitnessHealthFailure:" + ",".join(labels))
        return 1
    print(
        "WitnessHealthOK: in_use=%d max_concurrent=%d"
        % (body["requests"]["in_use"], body["requests"]["max_concurrent"])
    )
    return 0


def main(argv):
    if len(argv) != 2 or argv[1] not in ("poll", "check"):
        print("usage: witness_monitor.py poll|check", file=sys.stderr)
        return 2
    try:
        cfg = config_from_env(os.environ)
    except ConfigError as err:
        print("WitnessMonitorRefused:%s" % err, file=sys.stderr)
        return 2
    return poll(cfg) if argv[1] == "poll" else check(cfg)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
