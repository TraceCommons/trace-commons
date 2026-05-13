#!/usr/bin/env python3
"""Back-translation paraphrase helper for build-bakeoff-corpus.sh.

Reads JSONL on stdin where each line is {"original": "..."} and writes
JSONL on stdout where each line is {"original": "...", "paraphrase": "..."}.

Loads a local Qwen3-4B-Base checkpoint from the directory passed as
$BAKEOFF_PARAPHRASE_MODEL_PATH (also accepted as argv[1] for testing).

The pipeline is a single round-trip back-translation:
  1. Prompt the model: translate <original> into French.
  2. Prompt the model: translate the French rendering back to English.

The Python is deliberately minimal — heavy lifting is in transformers.
Fails loudly: any tokenizer / model / inference error exits non-zero with
a one-line CamelCase diagnostic on stderr so the bash caller can map it
into BakeoffCorpusParaphraseHelperFailed.

No network access; the model directory is expected to be local. We do
not log original text — failure messages identify rows by index only.
"""

from __future__ import annotations

import json
import os
import sys


def _bail(msg: str) -> "None":
    print(f"BakeoffParaphrase: {msg}", file=sys.stderr)
    sys.exit(1)


def _load_model(model_path: str):
    try:
        import torch  # noqa: F401  (imported for side effects / device probing)
        from transformers import AutoModelForCausalLM, AutoTokenizer
    except Exception as e:  # pylint: disable=broad-except
        _bail(f"transformers_import_failed reason={type(e).__name__}")
    if not os.path.isdir(model_path):
        _bail("model_path_not_a_directory")
    try:
        tok = AutoTokenizer.from_pretrained(model_path, local_files_only=True)
        mdl = AutoModelForCausalLM.from_pretrained(
            model_path,
            local_files_only=True,
            torch_dtype="auto",
            device_map="auto",
        )
    except Exception as e:  # pylint: disable=broad-except
        _bail(f"model_load_failed reason={type(e).__name__}")
    return tok, mdl


def _generate(tok, mdl, prompt: str, max_new_tokens: int = 512) -> str:
    import torch

    inputs = tok(prompt, return_tensors="pt").to(mdl.device)
    with torch.inference_mode():
        out = mdl.generate(
            **inputs,
            max_new_tokens=max_new_tokens,
            do_sample=False,
            temperature=1.0,
            pad_token_id=tok.eos_token_id,
        )
    text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=True)
    return text.strip()


def _paraphrase(tok, mdl, original: str) -> str:
    pivot_prompt = (
        "Translate the following English text into French. "
        "Output only the French translation, no commentary.\n\n"
        f"English: {original}\nFrench:"
    )
    pivot = _generate(tok, mdl, pivot_prompt)
    back_prompt = (
        "Translate the following French text into English. "
        "Output only the English translation, no commentary.\n\n"
        f"French: {pivot}\nEnglish:"
    )
    return _generate(tok, mdl, back_prompt)


def main() -> int:
    model_path = (
        sys.argv[1] if len(sys.argv) > 1 else os.environ.get("BAKEOFF_PARAPHRASE_MODEL_PATH", "")
    )
    if not model_path:
        _bail("model_path_unset")
    tok, mdl = _load_model(model_path)

    for idx, line in enumerate(sys.stdin):
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
            original = obj["original"]
        except Exception as e:  # pylint: disable=broad-except
            _bail(f"input_decode_failed row={idx} reason={type(e).__name__}")
        try:
            para = _paraphrase(tok, mdl, original)
        except Exception as e:  # pylint: disable=broad-except
            _bail(f"inference_failed row={idx} reason={type(e).__name__}")
        sys.stdout.write(json.dumps({"original": original, "paraphrase": para}) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
