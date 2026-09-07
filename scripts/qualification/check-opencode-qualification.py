#!/usr/bin/env python3
"""Fail closed on qualification pin or the deliberately empty owned catalog drift.

This is a narrow source-shape guard, not a Rust parser or proof about upstream
catalog entries. Refactoring either guarded expression requires updating these
checks and their mutation tests. It installs and executes no OpenCode binary.
"""
import ast
from pathlib import Path
import re
import sys


def script_pin(source):
    matches = []
    for node in ast.parse(source).body:
        if not isinstance(node, ast.If) or not isinstance(node.test, ast.Compare):
            continue
        values = node.test.comparators
        if len(values) != 1 or not isinstance(values[0], ast.Constant):
            continue
        version = values[0].value
        if not isinstance(version, str) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
            continue
        expected = ast.parse(
            'if subprocess.check_output([str(args.opencode), "--version"], text=True).strip() '
            f'!= {version!r}:\n    raise SystemExit("unqualified OpenCode version")'
        ).body[0]
        if ast.dump(node) == ast.dump(expected):
            matches.append(version)
    if len(matches) != 1:
        raise ValueError("missing or changed top-level binary-version refusal")
    return matches[0]


def check(reader, script, report, harness):
    pins = re.findall(r'^pub const QUALIFIED_VERSION: &str = "(\d+\.\d+\.\d+)";$', reader, re.M)
    documented = re.findall(r'Tested executable: OpenCode \*\*(\d+\.\d+\.\d+)\*\*', report)
    if len(pins) != 1 or len(documented) != 1:
        raise ValueError("missing or ambiguous reader/report pin")
    version = script_pin(script)
    if pins[0] != version or documented[0] != version:
        raise ValueError("reader, runtime refusal and tested report pins disagree")
    declarations = re.findall(r'\bfn\s+owned_agents\s*\(', harness)
    empty = re.findall(r'\bfn\s+owned_agents\s*\(\s*\)\s*->\s*Vec<AgentEntry>\s*\{\s*Vec::new\(\)\s*\}', harness)
    if len(declarations) != 1 or len(empty) != 1:
        raise ValueError("owned_agents body is no longer exactly the empty catalog; re-qualify before enabling")
    return version


def main():
    root = Path(__file__).resolve().parents[2]
    version = check(
        (root / "crates/trace-commons-contributor/src/source/opencode.rs").read_text(),
        (root / "scripts/qualification/opencode-routing.py").read_text(),
        (root / "docs/superpowers/reports/2026-09-07-opencode-export-qualification.md").read_text(),
        (root / "crates/trace-commons-contributor/src/daemon/harness.rs").read_text(),
    )
    print(f"OpenCode pins agree at {version}; owned catalog is empty")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, SyntaxError) as error:
        sys.exit(str(error))
