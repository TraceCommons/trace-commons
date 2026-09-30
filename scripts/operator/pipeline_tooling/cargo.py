"""Cargo test invocation with a zero-match guard.

`cargo test <filter>` reports "0 passed; 0 failed" when the filter matches
nothing -- a typo in a test name, a renamed test, or a moved module looks
exactly like a green run. `cargo_test` lists the matching tests first and
refuses to run the real command when the list is empty.

The list step builds the test binary, so a compile error surfaces there. Its
output goes to the step's protected log, and a nonzero exit fails as
`cargo_test_list_failed:<step>` with that log's path, never as a zero-match
filter.
"""

from __future__ import annotations

from . import environment
from .environment import run_child
from .errors import StepFailed, require


def cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
    trailer = []
    if exact:
        trailer.append("--exact")
    if ignored:
        trailer.append("--ignored")

    list_command = ["cargo", "test", *cargo_args, test_filter, "--", "--list", *trailer]
    returncode, output = environment._invoke(list_command, env=env, capture=True)
    log_path = run.log_path(step)
    log_path.write_text(output or "")
    if returncode != 0:
        raise StepFailed(step, returncode, log_path, label="cargo_test_list_failed")
    matched = sum(1 for line in (output or "").splitlines() if line.rstrip().endswith(": test"))
    require(matched > 0, "cargo_filter_matched_zero_tests")

    run_command = ["cargo", "test", *cargo_args, test_filter, "--", *trailer]
    run_child(run, step, run_command, env)
