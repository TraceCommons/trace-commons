"""Cargo test invocation with a zero-match guard.

`cargo test <filter>` reports "0 passed; 0 failed" when the filter matches
nothing -- a typo in a test name, a renamed test, or a moved module looks
exactly like a green run. `cargo_test` lists the matching tests first and
refuses to run the real command when the list is empty.
"""

from __future__ import annotations

from . import environment
from .environment import run_child
from .errors import require


def cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False):
    trailer = []
    if exact:
        trailer.append("--exact")
    if ignored:
        trailer.append("--ignored")

    list_command = ["cargo", "test", *cargo_args, test_filter, "--", "--list", *trailer]
    _, output = environment._invoke(list_command, env=env, capture=True)
    matched = sum(1 for line in (output or "").splitlines() if line.rstrip().endswith(": test"))
    require(matched > 0, "cargo_filter_matched_zero_tests")

    run_command = ["cargo", "test", *cargo_args, test_filter, "--", *trailer]
    run_child(run, step, run_command, env)
