"""Safe-label exceptions for the pipeline tooling.

Every exception raised here carries a single safe label: no URL, token,
command line, environment value, or trace body (repo convention: hash-only,
label-only operational surfaces). Callers print `str(error)` directly.
"""

from __future__ import annotations


class ToolingError(Exception):
    """A safe label. Never carries a URL, token, command line, or trace text."""


class StepFailed(ToolingError):
    """A child step that exited nonzero: `<label>:<step>`, its exit code,
    and the protected log that holds its output. `label` is `step_failed`
    unless the caller names the part of the step that failed (for example
    `cargo_test_list_failed`)."""

    def __init__(self, step, exit_code, log_path, *, label="step_failed"):
        super().__init__(f"{label}:{step}")
        self.step, self.exit_code, self.log_path = step, exit_code, log_path


def require(condition, label):
    if not condition:
        raise ToolingError(label)
