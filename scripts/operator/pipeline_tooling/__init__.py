"""Shared helpers for `scripts/operator/pipeline.py`.

Python 3 standard library only (no `psycopg`, no `pytest`: see the plan's
global constraints). Submodules:

- `errors`   -- `ToolingError`, `StepFailed`, `require`.
- `environment` -- `Run`, `Environment`, `Scenario`, `child_environment`,
  `run_child`.
- `cargo`    -- `cargo_test`, the zero-match-filter guard.
- `results`  -- `PipelineCheckResult` loading and validation.
- `checks`   -- check definitions (`CheckSpec`, `TEST_CHECKS`).
- `corpus`   -- `load_direct_corpus`, `load_pin`, `export_hf_corpus`.
"""
