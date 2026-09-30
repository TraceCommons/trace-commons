"""Safe-label exceptions for the pipeline tooling.

Every exception raised here carries a single safe label: no URL, token,
command line, environment value, or trace body (repo convention: hash-only,
label-only operational surfaces). Callers print `str(error)` directly.
"""

from __future__ import annotations


class ToolingError(Exception):
    """A safe label. Never carries a URL, token, command line, or trace text."""


class StepFailed(ToolingError):
    def __init__(self, step, exit_code, log_path):
        super().__init__(f"step_failed:{step}")
        self.step, self.exit_code, self.log_path = step, exit_code, log_path


def require(condition, label):
    if not condition:
        raise ToolingError(label)
