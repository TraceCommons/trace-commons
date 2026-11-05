#!/usr/bin/env python3
"""Self-tests for witness_monitor.py. Stdlib only; no network beyond loopback."""
import contextlib
import http.server
import io
import json
import os
import pathlib
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import witness_monitor as wm  # noqa: E402


def diagnostics(**over):
    """The /v1/diagnostics body, pinned field-for-field from witness PR #1975
    (witness_service/diagnostics.rs). Extra fields are tolerated; these six
    request fields are the contract this monitor reads."""
    requests = {
        "max_concurrent": 4,
        "in_use": 0,
        "completed_2xx": 10,
        "completed_4xx": 1,
        "completed_5xx": 0,
        "saturated_refusals": 0,
        "timed_out": 0,
    }
    requests.update(over.pop("requests", {}))
    body = {
        "schema": "witness-diagnostics-v1",
        "build": "trace-commons-server 0.1.0 (abc1234)",
        "witness_measurement": "mrtd:00+mrconfigid:11",
        "signing_address": "0x" + "00" * 19 + "01",
        "redaction_pipeline": "pipeline-alias",
        "uptime_secs": 1000,
        "requests": requests,
        "classifier": {
            "backend": "near-ai",
            "in_flight": 0,
            "outcomes": {"ok": 5, "timeout": 0},
            "latency_ms": {},
        },
    }
    body.update(over)
    return body


def cfg(**over):
    base = dict(
        url="http://127.0.0.1:1/v1/diagnostics",
        timeout_secs=2.0,
        saturated_delta=1,
        timed_out_delta=1,
        full_polls=3,
        records_path=None,
        state_path=None,
        max_records_bytes=1 << 20,
    )
    base.update(over)
    return wm.Config(**base)


class Evaluate(unittest.TestCase):
    def test_first_poll_has_no_baseline_so_counters_alone_do_not_alert(self):
        snap = diagnostics(requests={"saturated_refusals": 50, "timed_out": 9})
        alerts, state, _ = wm.evaluate(snap, None, cfg())
        self.assertEqual(alerts, [])
        self.assertEqual(state["saturated_refusals"], 50)

    def test_saturated_refusals_rising_alerts(self):
        _, state, _ = wm.evaluate(diagnostics(), None, cfg())
        snap = diagnostics(requests={"saturated_refusals": 2}, uptime_secs=1060)
        alerts, _, rec = wm.evaluate(snap, state, cfg())
        self.assertEqual(alerts, ["saturated_refusals_rose"])
        self.assertEqual(rec["saturated_delta"], 2)

    def test_timed_out_rising_alerts(self):
        _, state, _ = wm.evaluate(diagnostics(), None, cfg())
        snap = diagnostics(requests={"timed_out": 1}, uptime_secs=1060)
        alerts, _, _ = wm.evaluate(snap, state, cfg())
        self.assertEqual(alerts, ["timed_out_rose"])

    def test_delta_below_threshold_does_not_alert(self):
        c = cfg(saturated_delta=3)
        _, state, _ = wm.evaluate(diagnostics(), None, c)
        snap = diagnostics(requests={"saturated_refusals": 2}, uptime_secs=1060)
        alerts, _, _ = wm.evaluate(snap, state, c)
        self.assertEqual(alerts, [])

    def test_counter_reset_is_a_restart_not_an_alert_and_rebaselines(self):
        _, state, _ = wm.evaluate(
            diagnostics(requests={"saturated_refusals": 40}, uptime_secs=5000),
            None,
            cfg(),
        )
        snap = diagnostics(requests={"saturated_refusals": 0}, uptime_secs=30)
        alerts, new_state, rec = wm.evaluate(snap, state, cfg())
        self.assertEqual(alerts, [])
        self.assertTrue(rec["restarted"])
        self.assertEqual(new_state["saturated_refusals"], 0)

    def test_slots_full_needs_consecutive_polls(self):
        full = diagnostics(requests={"in_use": 4})
        state = None
        results = []
        for _ in range(3):
            alerts, state, _ = wm.evaluate(full, state, cfg())
            results.append(alerts)
        self.assertEqual(results, [[], [], ["slots_full_sustained"]])

    def test_a_non_full_poll_resets_the_streak(self):
        full = diagnostics(requests={"in_use": 4})
        state = None
        for _ in range(2):
            _, state, _ = wm.evaluate(full, state, cfg())
        _, state, _ = wm.evaluate(diagnostics(requests={"in_use": 3}), state, cfg())
        alerts, state, _ = wm.evaluate(full, state, cfg())
        self.assertEqual(alerts, [])

    def test_schema_mismatch_fails_closed(self):
        for bad in (
            diagnostics(schema="witness-diagnostics-v2"),
            {"schema": "witness-diagnostics-v1"},
            diagnostics(requests={"in_use": True}),
            diagnostics(requests={"max_concurrent": 0}),
            diagnostics(requests={"timed_out": "0"}),
            [],
        ):
            self.assertEqual(wm.validate(bad), "schema_mismatch", bad)
        self.assertIsNone(wm.validate(diagnostics()))


class Fetch(unittest.TestCase):
    def serve(self, handler_cls):
        server = http.server.HTTPServer(("127.0.0.1", 0), handler_cls)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        self.addCleanup(server.server_close)
        return f"http://127.0.0.1:{server.server_address[1]}/v1/diagnostics"

    def handler(self, status, body):
        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                data = body if isinstance(body, bytes) else json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *a):
                pass

        return H

    def test_ok(self):
        url = self.serve(self.handler(200, diagnostics()))
        failure, body = wm.fetch(url, 2.0)
        self.assertIsNone(failure)
        self.assertEqual(body["schema"], "witness-diagnostics-v1")

    def test_failure_labels(self):
        url = self.serve(self.handler(503, {}))
        self.assertEqual(wm.fetch(url, 2.0)[0], "http_5xx")
        url = self.serve(self.handler(404, {}))
        self.assertEqual(wm.fetch(url, 2.0)[0], "http_4xx")
        url = self.serve(self.handler(200, b"<html>"))
        self.assertEqual(wm.fetch(url, 2.0)[0], "body_not_json")
        self.assertEqual(
            wm.fetch("http://127.0.0.1:1/v1/diagnostics", 2.0)[0], "connection"
        )

    def test_failure_labels_never_carry_the_url(self):
        failure, _ = wm.fetch("http://127.0.0.1:1/secret-path-marker", 2.0)
        self.assertNotIn("secret-path-marker", failure)
        self.assertNotIn("127.0.0.1", failure)


class Poll(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp))
        self.cfg = cfg(
            url="http://witness.invalid/secret-path-marker",
            records_path=self.tmp / "records.jsonl",
            state_path=self.tmp / "state.json",
        )

    def poll(self, fetcher):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = wm.poll(self.cfg, fetcher=fetcher, now="2026-11-05T00:00:00Z")
        return code, out.getvalue()

    def records(self):
        lines = self.cfg.records_path.read_text().splitlines()
        return [json.loads(line) for line in lines]

    def test_healthy_poll_exits_0_and_appends_a_record(self):
        code, out = self.poll(lambda url, t: (None, diagnostics()))
        self.assertEqual(code, 0)
        rec = self.records()[0]
        self.assertEqual(rec["status"], "ok")
        self.assertEqual(rec["ts"], "2026-11-05T00:00:00Z")
        self.assertEqual(rec["alerts"], [])

    def test_unreachable_is_an_alert_never_healthy(self):
        code, out = self.poll(lambda url, t: ("timeout", None))
        self.assertEqual(code, 1)
        rec = self.records()[0]
        self.assertEqual(rec["status"], "alert")
        self.assertEqual(rec["alerts"], ["unreachable"])
        self.assertEqual(rec["failure"], "timeout")

    def test_schema_mismatch_is_an_alert(self):
        code, _ = self.poll(lambda url, t: (None, {"schema": "x"}))
        self.assertEqual(code, 1)
        self.assertEqual(self.records()[0]["alerts"], ["schema_mismatch"])

    def test_deltas_persist_across_polls(self):
        self.poll(lambda url, t: (None, diagnostics()))
        code, _ = self.poll(
            lambda url, t: (
                None,
                diagnostics(requests={"timed_out": 3}, uptime_secs=1060),
            )
        )
        self.assertEqual(code, 1)
        self.assertEqual(self.records()[1]["alerts"], ["timed_out_rose"])

    def test_unreachable_poll_keeps_the_baseline(self):
        self.poll(lambda url, t: (None, diagnostics()))
        self.poll(lambda url, t: ("connection", None))
        code, _ = self.poll(
            lambda url, t: (
                None,
                diagnostics(requests={"saturated_refusals": 1}, uptime_secs=1120),
            )
        )
        self.assertEqual(code, 1)
        self.assertEqual(self.records()[2]["alerts"], ["saturated_refusals_rose"])

    def test_corrupt_state_rebaselines_rather_than_crashing(self):
        self.cfg.state_path.write_text("{not json")
        code, _ = self.poll(lambda url, t: (None, diagnostics()))
        self.assertEqual(code, 0)

    def test_records_and_stdout_are_label_only(self):
        self.poll(lambda url, t: (None, diagnostics()))
        self.poll(lambda url, t: ("connection", None))
        code, out = self.poll(lambda url, t: (None, diagnostics()))
        blob = out + self.cfg.records_path.read_text() + self.cfg.state_path.read_text()
        for forbidden in (
            "secret-path-marker",
            "witness.invalid",
            "0x0000",
            "mrtd",
            "abc1234",
        ):
            self.assertNotIn(forbidden, blob)

    def test_records_file_rotates_at_the_cap(self):
        self.cfg = cfg(
            records_path=self.tmp / "records.jsonl",
            state_path=self.tmp / "state.json",
            max_records_bytes=200,
        )
        for _ in range(6):
            self.poll(lambda url, t: (None, diagnostics()))
        self.assertTrue((self.tmp / "records.jsonl.1").exists())
        self.assertLess((self.tmp / "records.jsonl").stat().st_size, 600)


class Check(unittest.TestCase):
    def run_check(self, fetcher):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = wm.check(cfg(), fetcher=fetcher)
        return code, out.getvalue().strip()

    def test_healthy(self):
        code, out = self.run_check(lambda u, t: (None, diagnostics()))
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("WitnessHealthOK"))

    def test_unreachable_and_missing_endpoint_fail(self):
        code, out = self.run_check(lambda u, t: ("http_4xx", None))
        self.assertEqual(code, 1)
        self.assertEqual(out, "WitnessHealthFailure:unreachable:http_4xx")

    def test_saturated_witness_fails_the_deploy_check(self):
        code, out = self.run_check(
            lambda u, t: (None, diagnostics(requests={"in_use": 4}))
        )
        self.assertEqual(code, 1)
        self.assertEqual(out, "WitnessHealthFailure:slots_full")

    def test_schema_mismatch_fails(self):
        code, out = self.run_check(lambda u, t: (None, {}))
        self.assertEqual(code, 1)
        self.assertEqual(out, "WitnessHealthFailure:schema_mismatch")


class ConfigFromEnv(unittest.TestCase):
    def test_defaults(self):
        c = wm.config_from_env({"TRACE_COMMONS_WITNESS_MONITOR_URL": "https://w.example/v1/diagnostics"})
        self.assertEqual(c.saturated_delta, 1)
        self.assertEqual(c.timed_out_delta, 1)
        self.assertEqual(c.full_polls, 3)
        self.assertEqual(c.timeout_secs, 10.0)

    def test_overrides(self):
        c = wm.config_from_env(
            {
                "TRACE_COMMONS_WITNESS_MONITOR_URL": "https://w.example/v1/diagnostics",
                "TRACE_COMMONS_WITNESS_MONITOR_FULL_POLLS": "5",
                "TRACE_COMMONS_WITNESS_MONITOR_SATURATED_DELTA": "4",
            }
        )
        self.assertEqual(c.full_polls, 5)
        self.assertEqual(c.saturated_delta, 4)

    def test_bad_config_is_refused_with_a_label_not_the_value(self):
        for env in (
            {},
            {"TRACE_COMMONS_WITNESS_MONITOR_URL": "file:///etc/passwd"},
            {
                "TRACE_COMMONS_WITNESS_MONITOR_URL": "https://w.example/x",
                "TRACE_COMMONS_WITNESS_MONITOR_FULL_POLLS": "0",
            },
            {
                "TRACE_COMMONS_WITNESS_MONITOR_URL": "https://w.example/x",
                "TRACE_COMMONS_WITNESS_MONITOR_SATURATED_DELTA": "abc-secret",
            },
        ):
            with self.assertRaises(wm.ConfigError) as ctx:
                wm.config_from_env(env)
            self.assertNotIn("secret", str(ctx.exception))
            self.assertNotIn("passwd", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
