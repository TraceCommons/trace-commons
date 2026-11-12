#!/usr/bin/env python3
"""Execute the production portal tail without a compositor or session bus."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class PortalGateTests(unittest.TestCase):
    def run_gate(self, prior_failure=0, owner=True, probe_rc=1, reply="UnknownMethod"):
        script = Path(__file__).with_name("weston-portal-verify-inner.sh").read_text()
        portal = script.index("\nPORTAL_UP=0\n")
        # Include every command after the rendering block, including a
        # premature exit accidentally inserted before the portal probe.
        boundary = script.rfind("\nfi\n", 0, portal) + len("\nfi\n")
        self.assertGreater(boundary, len("\nfi\n"))
        tail = script[boundary:]
        prelude = r'''
set -uo pipefail
FAIL="$PRIOR_FAILURE"
fail() { echo "FAIL: $1" >&2; FAIL=1; }
sleep() { :; }
gdbus() { echo "$OWNER_REPLY"; }
timeout() { echo probe >> "$PROBE_MARKER"; echo "$PROBE_REPLY"; return "$PROBE_RC"; }
'''
        with tempfile.TemporaryDirectory() as tmp:
            marker = Path(tmp) / "called"
            env = dict(os.environ, PRIOR_FAILURE=str(prior_failure),
                       OWNER_REPLY="(true,)" if owner else "(false,)",
                       PROBE_RC=str(probe_rc), PROBE_REPLY=reply, PROBE_MARKER=str(marker))
            result = subprocess.run(["bash", "-c", prelude + tail], env=env,
                                    capture_output=True, text=True, timeout=5)
            called = marker.exists()
        return result.returncode, called

    def test_success_reaches_live_portal_probe(self):
        self.assertEqual(self.run_gate(), (0, True))

    def test_render_failure_is_preserved_but_probe_still_runs(self):
        self.assertEqual(self.run_gate(prior_failure=1), (1, True))

    def test_missing_portal_fails_without_probing(self):
        self.assertEqual(self.run_gate(owner=False), (1, False))

    def test_probe_timeout_fails(self):
        self.assertEqual(self.run_gate(probe_rc=124, reply=""), (1, True))

    def test_absent_service_reply_fails(self):
        for reply in ("ServiceUnknown", "NameHasNoOwner", "was not provided by any .service files"):
            with self.subTest(reply=reply):
                self.assertEqual(self.run_gate(reply=reply), (1, True))

    def test_successful_method_reply_is_also_bounded_evidence(self):
        self.assertEqual(self.run_gate(probe_rc=0, reply="(objectpath '/request',)"), (0, True))


if __name__ == "__main__":
    unittest.main()
