#!/usr/bin/env python3
"""Smoke tests for the swival session-concat logic.

Runs standalone (no pytest dependency required):

    python3 scripts/operator/test_build_agent_traces_corpus.py

Exits non-zero on any failed assertion.
"""

from __future__ import annotations

import importlib.util
import io
import json
import sys
from pathlib import Path


def _load_module():
    here = Path(__file__).resolve().parent
    src = here / "build-agent-traces-corpus.py"
    spec = importlib.util.spec_from_file_location("build_agent_traces_corpus", src)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    sys.modules["build_agent_traces_corpus"] = mod
    spec.loader.exec_module(mod)
    return mod


M = _load_module()


def _events_to_jsonl(events: list[dict]) -> io.StringIO:
    return io.StringIO("\n".join(json.dumps(e) for e in events) + "\n")


def test_extract_event_text_string_content():
    out = M._extract_event_text({"message": {"content": "hello world"}})
    assert out == ["hello world"], out


def test_extract_event_text_chunk_list():
    event = {
        "message": {
            "content": [
                {"type": "text", "text": "alpha"},
                {"type": "tool_use", "input": {"x": 1}},  # no text field
                {"type": "text", "text": "  beta  "},
            ]
        }
    }
    assert M._extract_event_text(event) == ["alpha", "beta"]


def test_extract_event_text_top_level_content():
    event = {"content": "top-level snippet"}
    assert M._extract_event_text(event) == ["top-level snippet"]


def test_extract_event_text_both_message_and_top_level():
    event = {
        "message": {"content": "from-message"},
        "content": "from-top",
    }
    assert M._extract_event_text(event) == ["from-message", "from-top"]


def test_extract_event_text_empties_skipped():
    assert M._extract_event_text({"message": {"content": "   "}}) == []
    assert M._extract_event_text({"content": ""}) == []
    assert M._extract_event_text({}) == []


def test_swival_session_to_text_concats():
    events = [
        {"uuid": "a", "type": "user", "message": {"content": "first turn body"}},
        {
            "uuid": "b",
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": "assistant chunk one"},
                    {"type": "text", "text": "assistant chunk two"},
                ]
            },
        },
        {"uuid": "c", "type": "tool_result", "content": "tool output here"},
    ]
    lines = _events_to_jsonl(events)
    out = M.swival_session_to_text(lines)
    assert "first turn body" in out
    assert "assistant chunk one" in out
    assert "assistant chunk two" in out
    assert "tool output here" in out
    assert out.count("\n\n") >= 3, f"expected paragraph separators, got: {out!r}"


def test_swival_session_to_text_skips_malformed_lines():
    lines = io.StringIO(
        json.dumps({"message": {"content": "good line"}})
        + "\nNOT JSON\n"
        + json.dumps({"content": "another good line"})
        + "\n"
    )
    out = M.swival_session_to_text(lines)
    assert "good line" in out
    assert "another good line" in out


def test_swival_session_to_text_word_count_in_filter_band():
    # Construct a session that lands well inside the 200-2000 word band.
    body = ("word " * 60).strip()
    events = [{"message": {"content": body}} for _ in range(4)]
    out = M.swival_session_to_text(_events_to_jsonl(events))
    words = out.split()
    assert M.MIN_WORDS <= len(words) <= M.MAX_WORDS, len(words)


def test_collect_novel_texts_samples_deterministically(tmp_path=None):
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        # Build 12 sessions, each with ~240 words (inside the filter band).
        paths = []
        for i in range(12):
            body = (f"session{i}word " * 60).strip()
            events = [{"message": {"content": body}} for _ in range(4)]
            p = td_path / f"sess-{i:02d}.jsonl"
            p.write_text("\n".join(json.dumps(e) for e in events) + "\n")
            paths.append(p)

        a = M.collect_novel_texts(iter(paths), M.swival_session_to_text, count=5, seed=42, pool_cap=20)
        b = M.collect_novel_texts(iter(paths), M.swival_session_to_text, count=5, seed=42, pool_cap=20)
        assert a == b, "sampling should be deterministic at a fixed seed"
        assert len(a) == 5
        assert all(t for t in a)


def main() -> int:
    tests = [v for k, v in globals().items() if k.startswith("test_") and callable(v)]
    failures = []
    for fn in tests:
        try:
            fn()
            print(f"PASS {fn.__name__}")
        except AssertionError as exc:
            failures.append((fn.__name__, repr(exc)))
            print(f"FAIL {fn.__name__}: {exc!r}")
        except Exception as exc:  # noqa: BLE001
            failures.append((fn.__name__, repr(exc)))
            print(f"ERROR {fn.__name__}: {exc!r}")
    if failures:
        print(f"\n{len(failures)} failure(s)")
        return 1
    print(f"\nAll {len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
