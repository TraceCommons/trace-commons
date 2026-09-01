"""Contract tests for the privacy-filter shim.

The offset tests are the important ones. The Rust `apply_spans` treats
start/end as CODEPOINT offsets. If this shim ever emits byte offsets, redaction
lands on the wrong characters: the PII survives and unrelated text is
destroyed, while the call reports success.

These tests stub the model so they stay fast and deterministic. The real model
is exercised separately at deploy time.
"""

from fastapi.testclient import TestClient

import app as shim

# 'Ping 大三 about bob@example.com today'
# The two CJK characters are 3 bytes each, so the email's codepoint offset and
# byte offset differ by 4. That gap is the whole point of these tests.
TEXT = "Ping 大三 about bob@example.com today"
EMAIL = "bob@example.com"
CODEPOINT_START = TEXT.index(EMAIL)
BYTE_START = len(TEXT[:CODEPOINT_START].encode("utf-8"))


def test_the_fixture_actually_distinguishes_the_two_conventions():
    """Guard the guard: if these ever coincide, the tests below prove nothing.

    'Ping ' is 5 characters, the two CJK characters are 1 character but 3 bytes
    each, then ' about ' is 7. So the email starts at codepoint 14 and byte 18:
    a 4-byte gap, one per extra byte of the two multi-byte characters.
    """
    assert CODEPOINT_START != BYTE_START
    assert CODEPOINT_START == 14
    assert BYTE_START == 18
    assert BYTE_START - CODEPOINT_START == 4


def test_spans_are_codepoint_offsets_not_byte_offsets(monkeypatch):
    monkeypatch.setattr(
        shim,
        "detect_spans",
        lambda text: [
            ("private_email", CODEPOINT_START, CODEPOINT_START + len(EMAIL), EMAIL)
        ],
    )
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify",
        json={"model": "openai/privacy-filter", "input": TEXT},
    ).json()

    span = body["data"][0]["spans"][0]
    assert span["start"] == CODEPOINT_START, "codepoint offset expected"
    assert span["start"] != BYTE_START, "byte offset leaked into the response"
    assert TEXT[span["start"] : span["end"]] == EMAIL


def test_byte_offsets_are_rejected_rather_than_served(monkeypatch):
    """A model emitting byte offsets must fail the request, not be passed on.

    This is the failure that would otherwise be silent and would leak PII.
    """
    monkeypatch.setattr(
        shim,
        "detect_spans",
        lambda text: [("private_email", BYTE_START, BYTE_START + len(EMAIL), EMAIL)],
    )
    client = TestClient(shim.app)
    response = client.post(
        "/v1/privacy/classify",
        json={"model": "openai/privacy-filter", "input": TEXT},
    )
    assert response.status_code == 500
    assert "offset convention mismatch" in response.json()["detail"]


def test_ascii_only_text_round_trips(monkeypatch):
    text = "contact bob@example.com today"
    start = text.index(EMAIL)
    monkeypatch.setattr(
        shim,
        "detect_spans",
        lambda t: [("private_email", start, start + len(EMAIL), EMAIL)],
    )
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify",
        json={"model": "openai/privacy-filter", "input": text},
    ).json()
    span = body["data"][0]["spans"][0]
    assert text[span["start"] : span["end"]] == EMAIL


def test_all_eight_categories_pass_through_unmapped(monkeypatch):
    """The taxonomy matches the Rust allowlist, so no translation happens.

    If a future model version renames a label, it must surface rather than be
    silently coerced into a known one.
    """
    categories = [
        "private_person",
        "private_address",
        "private_email",
        "private_phone",
        "private_url",
        "private_date",
        "account_number",
        "secret",
    ]
    text = "x" * len(categories)
    monkeypatch.setattr(
        shim,
        "detect_spans",
        lambda t: [(c, i, i + 1, "x") for i, c in enumerate(categories)],
    )
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify",
        json={"model": "openai/privacy-filter", "input": text},
    ).json()
    assert [s["category"] for s in body["data"][0]["spans"]] == categories


def test_empty_input_returns_an_empty_span_list_not_an_error():
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify", json={"model": "openai/privacy-filter", "input": ""}
    ).json()
    assert body == {"data": [{"spans": []}]}


def test_whitespace_only_input_is_treated_as_empty():
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify",
        json={"model": "openai/privacy-filter", "input": "   \n\t "},
    ).json()
    assert body == {"data": [{"spans": []}]}


def test_clean_text_returns_an_entry_with_no_spans(monkeypatch):
    """Must be `data: [{spans: []}]`, never `data: []`.

    The Rust adapter fails closed on an empty data array, so a clean field has
    to come back as one entry holding zero spans.
    """
    monkeypatch.setattr(shim, "detect_spans", lambda text: [])
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify",
        json={"model": "openai/privacy-filter", "input": "nothing sensitive here"},
    ).json()
    assert body == {"data": [{"spans": []}]}


def test_healthz_reports_the_loaded_model():
    client = TestClient(shim.app)
    body = client.get("/healthz").json()
    assert body["status"] == "ok"
    assert body["model"]
