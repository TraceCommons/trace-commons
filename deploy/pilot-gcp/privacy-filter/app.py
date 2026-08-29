"""Loopback privacy-classifier serving openai/privacy-filter.

Reproduces NEAR AI Cloud's POST /v1/privacy/classify wire shape so the Rust
side shares one span decoder across both backends.

OFFSET CONVENTION: spans are CODEPOINT offsets.

This is not an assumption. opf/_core/runtime.py builds each span as

    for label_idx, start, end in predicted_char_spans:
        if not (0 <= start < end <= len(source_text)):
            continue
        detected.append(DetectedSpan(..., text=source_text[start:end], ...))

`source_text[start:end]` is Python string slicing and `len(source_text)` is a
codepoint count, so the offsets are codepoints by construction. The Rust
`apply_spans` expects exactly that.

We do not rely on that reading. Because DetectedSpan carries the matched text,
every span is verified here against the input before it is serialized: if
`text[start:end]` does not reproduce the span's own text, the request fails
closed. A byte-for-codepoint slip would otherwise redact the wrong characters,
leave the PII in place, and report success -- so it is checked, every call,
rather than trusted.
"""

import os

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

MODEL_ID = os.environ.get("PRIVACY_FILTER_MODEL", "openai/privacy-filter")
DEVICE = os.environ.get("PRIVACY_FILTER_DEVICE", "cpu")
CHECKPOINT = os.environ.get("OPF_CHECKPOINT")

# The model reports no per-span confidence. The Rust decoder uses `score` only
# to pick a winner when two spans overlap, so a constant is correct rather than
# merely convenient: it makes the tie-break first-wins and deterministic. opf
# already discards overlapping spans of the same label upstream.
CONSTANT_SPAN_SCORE = 1.0

app = FastAPI()
_model = None


def _load():
    """Load the checkpoint once, from local disk only.

    Weights are staged at deploy time. A boot that silently reaches the network
    is exactly the dependency the fail-closed convention forbids, so the unit
    also sets HF_HUB_OFFLINE=1.
    """
    global _model
    if _model is None:
        from opf import OPF

        _model = OPF(model=CHECKPOINT) if CHECKPOINT else OPF()
    return _model


def detect_spans(text):
    """Return [(category, start_codepoint, end_codepoint, matched_text)].

    Seam for tests: test_app.py monkeypatches this so the wire contract can be
    verified without loading 1.5B parameters.
    """
    result = _load().redact(text)
    return [(s.label, s.start, s.end, s.text) for s in result.detected_spans]


class ClassifyRequest(BaseModel):
    model: str
    input: str


@app.post("/v1/privacy/classify")
def classify(req: ClassifyRequest):
    if not req.input.strip():
        return {"data": [{"spans": []}]}

    spans = []
    for category, start, end, matched in detect_spans(req.input):
        # The invariant, enforced per span. Slicing the input with the offsets
        # we are about to hand downstream must reproduce the text the model
        # says it matched. If it does not, the offsets do not mean what the
        # consumer will assume, and redaction would land on the wrong
        # characters while reporting success.
        if req.input[start:end] != matched:
            raise HTTPException(
                status_code=500,
                detail=(
                    "span offset convention mismatch: slicing the input by the "
                    "reported offsets did not reproduce the matched text"
                ),
            )
        spans.append(
            {
                "category": category,
                "start": start,
                "end": end,
                "score": CONSTANT_SPAN_SCORE,
            }
        )

    return {"data": [{"spans": spans}]}


@app.get("/healthz")
def healthz():
    return {"status": "ok", "model": MODEL_ID, "device": DEVICE}
