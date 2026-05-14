#!/usr/bin/env python3
"""Per-token rarity prototype — Phase A.5 first experiment.

Reads a bake-off corpus tarball (`.tar.zst` per the format produced by
`crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs`), runs a
small language model over the novel and duplicate slices, and computes two
discrimination AUCs side-by-side:

  - aggregate-perplexity AUC: the existing metric (mean negative log-likelihood
    across all tokens, exponentiated).
  - per-token rarity AUC: average of the K lowest token logprobs in the trace,
    negated and exponentiated. The hypothesis is that genuinely novel content
    has a few highly surprising tokens even when the rest is fluent.

This is research scaffolding, not production code. The intended production
landing site for the metric is the `trace-commons-gate-enclave::perplexity_local`
module — see `docs/superpowers/specs/2026-05-14-a5-perplexity-replacement-design.md`
for the design context.

Usage:
    python3 scripts/research/per-token-rarity-prototype.py \\
        --corpus=/path/to/corpus.tar.zst \\
        --model=Qwen/Qwen3-8B-Base \\
        --k=10 \\
        --out=/tmp/rarity-report.json

For CI / smoke validation, pass the committed synthetic fixture and a small
CPU model:

    python3 scripts/research/per-token-rarity-prototype.py \\
        --corpus=scripts/research/fixtures/synthetic-corpus.tar.zst \\
        --model=gpt2 --k=4
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import os
import sys
import tarfile
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import List, Sequence, Tuple


# ---------------------------------------------------------------------------
# Corpus loading
# ---------------------------------------------------------------------------


@dataclass
class LoadedCorpus:
    novel: List[str]
    duplicate: List[str]
    corpus_sha256: str  # sha256 of the input tarball file


def _decompress_zst(path: Path) -> bytes:
    """Decompress a .zst file using `zstandard` if installed, else the zstd CLI."""
    raw = path.read_bytes()
    try:
        import zstandard as zstd  # type: ignore

        dctx = zstd.ZstdDecompressor()
        return dctx.decompress(raw, max_output_size=1 << 30)
    except ImportError:
        # Fall back to the zstd CLI. Operator hosts typically have it; CI
        # installs it via apt/brew. We deliberately avoid making zstandard
        # mandatory because the Python package isn't always pre-installed.
        import shutil
        import subprocess

        if shutil.which("zstd") is None:
            raise SystemExit(
                "neither the `zstandard` Python package nor the `zstd` CLI "
                "is available; install one of them to load .tar.zst corpora"
            )
        proc = subprocess.run(
            ["zstd", "-d", "-c", str(path)],
            capture_output=True,
            check=True,
        )
        return proc.stdout


def _sha256_label(chunks: Sequence[bytes]) -> str:
    h = hashlib.sha256()
    for c in chunks:
        h.update(c)
    return "sha256:" + h.hexdigest()


def load_corpus(tarball: Path) -> LoadedCorpus:
    """Mirror the Rust loader in `bakeoff_corpus.rs`.

    Validates the manifest sha256s the same way: novel/duplicate slices are
    `.txt` files in sorted-filename order, concatenated then hashed.
    """
    file_sha = hashlib.sha256(tarball.read_bytes()).hexdigest()
    decompressed = _decompress_zst(tarball)
    with tarfile.open(fileobj=io.BytesIO(decompressed), mode="r") as tf:
        members = tf.getmembers()
        files = {m.name.lstrip("./"): m for m in members if m.isfile()}

        manifest_member = files.get("manifest.json")
        if manifest_member is None:
            raise SystemExit("corpus tarball missing manifest.json")
        manifest = json.loads(tf.extractfile(manifest_member).read().decode("utf-8"))

        def _slice(prefix: str) -> Tuple[List[str], List[bytes]]:
            names = sorted(n for n in files if n.startswith(prefix + "/"))
            texts: List[str] = []
            bodies: List[bytes] = []
            for n in names:
                raw = tf.extractfile(files[n]).read()
                bodies.append(raw)
                texts.append(raw.decode("utf-8"))
            return texts, bodies

        novel_texts, novel_bodies = _slice("novel")
        dup_texts, dup_bodies = _slice("duplicate")

        if _sha256_label(novel_bodies) != manifest["novel_sha256"]:
            raise SystemExit(
                "novel slice sha256 mismatch vs manifest; refusing to load"
            )
        if _sha256_label(dup_bodies) != manifest["duplicate_sha256"]:
            raise SystemExit(
                "duplicate slice sha256 mismatch vs manifest; refusing to load"
            )

    return LoadedCorpus(
        novel=novel_texts,
        duplicate=dup_texts,
        corpus_sha256="sha256:" + file_sha,
    )


# ---------------------------------------------------------------------------
# Scoring
# ---------------------------------------------------------------------------


def _load_model(model_id: str):
    """Lazy import so corpus-only commands don't drag in torch."""
    try:
        import torch  # type: ignore
        from transformers import AutoModelForCausalLM, AutoTokenizer  # type: ignore
    except ImportError as e:
        raise SystemExit(
            f"transformers/torch import failed: {e}. Install with "
            "`pip install transformers torch` (operator-managed)."
        )

    tokenizer = AutoTokenizer.from_pretrained(model_id)
    model = AutoModelForCausalLM.from_pretrained(model_id)
    model.eval()
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token
    return torch, model, tokenizer


def token_logprobs(torch, model, tokenizer, text: str) -> List[float]:
    """Return per-token logprob (natural log) of the actual next tokens.

    Returns an empty list for inputs that tokenize to fewer than 2 tokens
    (no shift-by-one prediction is possible).
    """
    ids = tokenizer(text, return_tensors="pt", truncation=True, max_length=2048).input_ids
    if ids.shape[1] < 2:
        return []
    with torch.no_grad():
        logits = model(ids).logits  # [1, T, V]
    # Standard causal-LM shift: predict ids[:, 1:] from logits[:, :-1, :].
    shift_logits = logits[0, :-1, :]
    shift_targets = ids[0, 1:]
    log_softmax = torch.log_softmax(shift_logits.float(), dim=-1)
    chosen = log_softmax.gather(1, shift_targets.unsqueeze(-1)).squeeze(-1)
    return chosen.tolist()


def aggregate_perplexity(logprobs: Sequence[float]) -> float:
    """Standard perplexity: exp(-mean logprob). Empty -> inf so it sorts high."""
    if not logprobs:
        return float("inf")
    mean = sum(logprobs) / len(logprobs)
    return math.exp(-mean)


def per_token_rarity(logprobs: Sequence[float], k: int) -> float:
    """Rarity = exp(-mean of the K lowest logprobs).

    K is capped at len(logprobs). Empty input -> inf, mirroring perplexity.
    """
    if not logprobs:
        return float("inf")
    k_eff = min(k, len(logprobs))
    rarest = sorted(logprobs)[:k_eff]
    mean = sum(rarest) / len(rarest)
    return math.exp(-mean)


# ---------------------------------------------------------------------------
# Discrimination AUC (mirrors `bakeoff_metrics::discrimination_auc`)
# ---------------------------------------------------------------------------


def discrimination_auc(novel: Sequence[float], duplicate: Sequence[float]) -> float:
    """Mann–Whitney U convention. Ties contribute 0.5. Empty -> 0.5.

    Ports `discrimination_auc` from
    `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs` so the
    prototype's AUC numbers are directly comparable to the bake-off binary.
    """
    if not novel or not duplicate:
        return 0.5
    wins = 0.0
    for n in novel:
        for d in duplicate:
            if n > d:
                wins += 1.0
            elif n == d:
                wins += 0.5
    return wins / (len(novel) * len(duplicate))


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _recommendation(delta: float) -> str:
    if delta >= 0.05:
        return "investigate further (delta >= 0.05 AUC)"
    if delta > 0.0:
        return "marginal signal (delta < 0.05 AUC; below the spec threshold)"
    return "rarity does not add signal vs aggregate perplexity"


def main(argv: List[str]) -> int:
    p = argparse.ArgumentParser(description="Per-token rarity prototype (Phase A.5).")
    p.add_argument("--corpus", required=True, type=Path, help="path to corpus .tar.zst")
    p.add_argument("--model", default="gpt2", help="HF repo id or local path")
    p.add_argument("--k", type=int, default=10, help="number of rarest tokens to average")
    p.add_argument("--out", type=Path, default=None, help="optional JSON report path")
    p.add_argument(
        "--max-traces",
        type=int,
        default=0,
        help="if > 0, cap each slice at this many traces (debug aid)",
    )
    args = p.parse_args(argv)

    if args.k < 1:
        print("--k must be >= 1", file=sys.stderr)
        return 2

    corpus = load_corpus(args.corpus)
    novel = corpus.novel
    duplicate = corpus.duplicate
    if args.max_traces > 0:
        novel = novel[: args.max_traces]
        duplicate = duplicate[: args.max_traces]

    print("=== Per-token rarity prototype ===")
    print(f"Model: {args.model}")
    print(f"Corpus: {corpus.corpus_sha256}")
    print(f"Novel slice: {len(novel)} entries")
    print(f"Duplicate slice: {len(duplicate)} entries")
    print()

    torch, model, tokenizer = _load_model(args.model)

    def score_slice(texts: Sequence[str]) -> Tuple[List[float], List[float]]:
        ppls: List[float] = []
        rarities: List[float] = []
        for t in texts:
            lp = token_logprobs(torch, model, tokenizer, t)
            ppls.append(aggregate_perplexity(lp))
            rarities.append(per_token_rarity(lp, args.k))
        return ppls, rarities

    novel_ppl, novel_rarity = score_slice(novel)
    dup_ppl, dup_rarity = score_slice(duplicate)

    ppl_auc = discrimination_auc(novel_ppl, dup_ppl)
    rarity_auc = discrimination_auc(novel_rarity, dup_rarity)
    delta = rarity_auc - ppl_auc

    print(f"Aggregate perplexity AUC: {ppl_auc:.4f}")
    print(f"Per-token rarity AUC (K={args.k}): {rarity_auc:.4f}")
    print()
    sign = "+" if delta >= 0 else ""
    print(f"Delta: {sign}{delta:.4f} (rarity - aggregate)")
    print()
    print(f"Recommendation: {_recommendation(delta)}")

    if args.out is not None:
        report = {
            "model": args.model,
            "corpus_sha256": corpus.corpus_sha256,
            "k": args.k,
            "novel_count": len(novel),
            "duplicate_count": len(duplicate),
            "aggregate_perplexity_auc": ppl_auc,
            "per_token_rarity_auc": rarity_auc,
            "delta": delta,
            "recommendation": _recommendation(delta),
            "novel_scores": {
                "perplexity": novel_ppl,
                "rarity": novel_rarity,
            },
            "duplicate_scores": {
                "perplexity": dup_ppl,
                "rarity": dup_rarity,
            },
        }
        args.out.write_text(json.dumps(report, indent=2))
        print()
        print(f"Wrote report to {args.out}")

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
