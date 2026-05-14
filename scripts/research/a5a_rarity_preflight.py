#!/usr/bin/env python3
# DEFERRED (2026-05-14, post-A2.6 partial): Qwen 3.6 27B Dense landed AUC
# 0.936 on A2.6's agent-traces corpus, well above 0.5. Per the A2.6 outcome
# map, this triggers A2.7 (perplexity floor recalibration), NOT A.5. A.5
# (and therefore this pre-flight) is DEFERRED. Gemma 4 31B still in flight;
# this script would only re-activate if Gemma confirms a categorical failure
# mode the 27B somehow masked (very unlikely). See
# project_perplexity_size_pattern.md memory for the size-pattern finding —
# perplexity is not broken, it just needs a model big enough. Retained for
# reference, not execution.
"""A.5a per-token rarity pre-flight — focused one-candidate rerun.

Per the architect verdict captured in
`docs/superpowers/specs/2026-05-14-a5a-rarity-real-scorer-design.md`
("Pre-flight experiment (mandatory before production wiring)"), this script
re-runs a single A2.6 candidate over the committed A2.6 corpus in
**rarity-only** mode and emits per-trace rarity scores plus a per-K AUC
sweep over K ∈ {4, 8, 16, 32}. The intent is to cheaply confirm that
per-token rarity does not share aggregate perplexity's failure mode on this
corpus shape **before** investing in the real-scorer wiring slice
(modifying `LocalPerplexityScorer` to expose un-aggregated logprobs through
the `TokenRarityScorer` trait).

This is investigation scaffolding, not production code. It deliberately
does not modify any Rust crate. The spec endorses a Python script here:

  > A one-off Python script consuming the mistralrs Raw channel directly
  > and aggregating rarity offline is sufficient. Keep the script in
  > `scripts/research/` and treat its output as evidence, not production
  > code.

Path chosen: Python with `transformers` + `torch` (logprobs via
`output_scores`/log-softmax of the forward pass), not the Rust binary path.
Reason: the Rust path would require modifying
`crates/tracedao-gate-enclave/src/perplexity_local.rs` to surface
un-aggregated logprobs out of `LocalPerplexityScorer::async_score` — which
the spec explicitly classifies as the **production wiring** (the actual
A.5a build), not the pre-flight. The pre-flight is gated on the AUC result
of this script; building the production wiring first inverts the
decision.

Usage:
    python3 scripts/research/a5a_rarity_preflight.py \\
        --candidate llama-3.1-8b-instruct \\
        --corpus scripts/operator/fixtures/corpus-a26.tar.zst \\
        --k-values "4,8,16,32" \\
        --report-out ~/preflight-llama.json

CPU/smoke mode (no transformers required):
    python3 scripts/research/a5a_rarity_preflight.py \\
        --candidate synthetic-test \\
        --corpus scripts/research/fixtures/synthetic-corpus.tar.zst \\
        --k-values "4,8,16,32" \\
        --report-out /tmp/preflight-smoke.json \\
        --mock-logprobs-seed 42

Hash-only audit: this script never logs raw token strings, token ids, or
trace bodies. Operational output is limited to slice counts, AUC values,
elapsed seconds, throughput, the corpus sha256, and the candidate id /
model revision (operator-set labels).
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import os
import random
import sys
import tarfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Sequence, Tuple


# ---------------------------------------------------------------------------
# Corpus loading — mirrors `bakeoff_corpus.rs::load_corpus`.
# ---------------------------------------------------------------------------


@dataclass
class ParaphrasePair:
    original: str
    paraphrase: str


@dataclass
class LoadedCorpus:
    novel: List[str]
    duplicate: List[str]
    paraphrase: List[ParaphrasePair]
    corpus_sha256: str  # sha256 of the input tarball file


def _decompress_zst(path: Path) -> bytes:
    raw = path.read_bytes()
    try:
        import zstandard as zstd  # type: ignore

        dctx = zstd.ZstdDecompressor()
        return dctx.decompress(raw, max_output_size=1 << 30)
    except ImportError:
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
    """Load a bake-off corpus tarball. Verifies all three slice sha256s.

    Format is documented in
    `crates/tracedao-server/src/bin/gate_calibrate/bakeoff_corpus.rs`. Novel
    and duplicate slices are `.txt` files in sorted-filename order;
    paraphrase is a single `paraphrase/paraphrase.jsonl` with one
    `{original, paraphrase}` object per line.
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

        jsonl_member = files.get("paraphrase/paraphrase.jsonl")
        if jsonl_member is None:
            raise SystemExit("corpus tarball missing paraphrase/paraphrase.jsonl")
        jsonl_bytes = tf.extractfile(jsonl_member).read()
        if _sha256_label([jsonl_bytes]) != manifest["paraphrase_sha256"]:
            raise SystemExit(
                "paraphrase slice sha256 mismatch vs manifest; refusing to load"
            )
        paraphrase_pairs: List[ParaphrasePair] = []
        for lineno, line in enumerate(jsonl_bytes.decode("utf-8").splitlines()):
            if not line.strip():
                continue
            try:
                pl = json.loads(line)
            except json.JSONDecodeError as e:
                raise SystemExit(
                    f"paraphrase.jsonl line {lineno} parse error: {e}"
                ) from None
            paraphrase_pairs.append(
                ParaphrasePair(original=pl["original"], paraphrase=pl["paraphrase"])
            )

    return LoadedCorpus(
        novel=novel_texts,
        duplicate=dup_texts,
        paraphrase=paraphrase_pairs,
        corpus_sha256="sha256:" + file_sha,
    )


# ---------------------------------------------------------------------------
# Scoring — real (transformers) and mocked.
# ---------------------------------------------------------------------------


def _resolve_candidate_path(candidate: str) -> str:
    """Map a candidate id to a local model path or pass-through HF repo id.

    Mirrors the bake-off binary's lookup pattern: a local checkpoint under
    `~/models/<candidate>/` takes precedence; if absent we pass the
    candidate id straight to `from_pretrained` which lets the operator hand
    in an HF repo id (with credentials configured out-of-band).
    """
    local = Path.home() / "models" / candidate
    if local.is_dir():
        return str(local)
    return candidate


def _model_revision(model_path_or_id: str) -> str:
    """Best-effort model provenance label for the report header.

    Local path: sha256 of `config.json` (cheap stable identifier; full
    safetensors hashing is too slow). HF repo id: pass-through string;
    operator records the revision out-of-band.
    """
    cfg_path = Path(model_path_or_id) / "config.json"
    if cfg_path.is_file():
        return "config_sha256:" + hashlib.sha256(cfg_path.read_bytes()).hexdigest()
    return "hf_repo:" + model_path_or_id


def _load_real_scorer(candidate: str):
    """Lazy import; returns a closure `(text) -> List[float]` of per-token
    logprobs of the realized next tokens. Empty list for inputs that
    tokenize to fewer than 2 tokens.
    """
    try:
        import torch  # type: ignore
        from transformers import AutoModelForCausalLM, AutoTokenizer  # type: ignore
    except ImportError as e:
        raise SystemExit(
            f"transformers/torch import failed: {e}. Install with "
            "`pip install transformers torch` on the Lambda host."
        )

    resolved = _resolve_candidate_path(candidate)
    tokenizer = AutoTokenizer.from_pretrained(resolved)
    model = AutoModelForCausalLM.from_pretrained(resolved)
    model.eval()
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    def score(text: str) -> List[float]:
        ids = tokenizer(
            text, return_tensors="pt", truncation=True, max_length=4096
        ).input_ids
        if ids.shape[1] < 2:
            return []
        with torch.no_grad():
            logits = model(ids).logits  # [1, T, V]
        shift_logits = logits[0, :-1, :]
        shift_targets = ids[0, 1:]
        lp = torch.log_softmax(shift_logits.float(), dim=-1)
        chosen = lp.gather(1, shift_targets.unsqueeze(-1)).squeeze(-1)
        return chosen.tolist()

    revision = _model_revision(resolved)
    return score, revision


def _mock_logprobs_for_text(text: str, seed: int) -> List[float]:
    """Deterministic synthetic logprobs derived from a sha256(text|seed) PRNG.

    Used only by the smoke test path. The mapping is hash-stable so re-runs
    against the same input + seed produce identical numbers — that's the
    determinism property the smoke test asserts.

    Logprob distribution: Gaussian(mean=-3.0, std=2.0), with a length cap
    of 128 tokens (one logprob per byte modulo a length factor) so the
    K-rarest sort over K in {4,8,16,32} is non-degenerate even on the
    synthetic fixture's short traces.
    """
    h = hashlib.sha256(f"{seed}:{text}".encode("utf-8")).digest()
    # Derive a stable token count in [2, 128] from the first byte so the
    # K-sort has enough headroom for K=32.
    n_tokens = 2 + (h[0] % 127)
    rng = random.Random(int.from_bytes(h[:16], "big"))
    return [rng.gauss(-3.0, 2.0) for _ in range(n_tokens)]


def _load_mock_scorer(seed: int):
    def score(text: str) -> List[float]:
        return _mock_logprobs_for_text(text, seed)

    return score, f"mock_logprobs:seed={seed}"


# ---------------------------------------------------------------------------
# Metric aggregation.
# ---------------------------------------------------------------------------


def aggregate_perplexity(logprobs: Sequence[float]) -> float:
    if not logprobs:
        return float("inf")
    mean = sum(logprobs) / len(logprobs)
    return math.exp(-mean)


def per_token_rarity(logprobs: Sequence[float], k: int) -> float:
    """Rarity = exp(-mean of the K lowest logprobs). Empty -> inf."""
    if not logprobs:
        return float("inf")
    k_eff = min(k, len(logprobs))
    rarest = sorted(logprobs)[:k_eff]
    mean = sum(rarest) / len(rarest)
    return math.exp(-mean)


def to_micros(v: float) -> int:
    """Saturating conversion to fixed-point micros, matching the Rust
    `saturating_micros` helper in `perplexity_local.rs`. Non-finite or
    negative inputs collapse to 0 (fail-closed)."""
    if not math.isfinite(v) or v < 0.0:
        return 0
    scaled = v * 1_000_000.0
    if scaled >= float(2**64 - 1):
        return 2**64 - 1
    return int(scaled)


def discrimination_auc(novel: Sequence[float], duplicate: Sequence[float]) -> float:
    """Mann–Whitney U, ties = 0.5. Ports `bakeoff_metrics::discrimination_auc`."""
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
# Pre-flight runner.
# ---------------------------------------------------------------------------


def parse_k_values(raw: str) -> List[int]:
    out: List[int] = []
    for piece in raw.split(","):
        piece = piece.strip()
        if not piece:
            continue
        k = int(piece)
        if k < 1:
            raise ValueError(f"K must be >= 1, got {k}")
        out.append(k)
    if not out:
        raise ValueError("--k-values produced an empty list")
    return out


def run_preflight(
    candidate: str,
    corpus: LoadedCorpus,
    k_values: Sequence[int],
    score_fn,
    model_revision: str,
) -> dict:
    """Score every entry once, aggregate rarity at each K, and build the
    report. The expensive forward pass is amortized across all K values —
    we collect logprobs per entry and re-aggregate locally.
    """
    paraphrase_texts = [p.paraphrase for p in corpus.paraphrase]

    novel_lp: List[List[float]] = []
    duplicate_lp: List[List[float]] = []
    paraphrase_lp: List[List[float]] = []

    started = time.monotonic()
    total_tokens = 0

    def _score_slice(texts: Iterable[str], sink: List[List[float]]) -> None:
        nonlocal total_tokens
        for t in texts:
            lp = score_fn(t)
            sink.append(lp)
            total_tokens += len(lp)

    _score_slice(corpus.novel, novel_lp)
    _score_slice(corpus.duplicate, duplicate_lp)
    _score_slice(paraphrase_texts, paraphrase_lp)

    elapsed = time.monotonic() - started

    # Baseline aggregate perplexity per slice (one number per entry) — kept
    # alongside the rarity sweep so the operator can compare directly to the
    # A2.6 perplexity AUC without consulting a second report.
    novel_ppl = [aggregate_perplexity(lp) for lp in novel_lp]
    duplicate_ppl = [aggregate_perplexity(lp) for lp in duplicate_lp]
    paraphrase_ppl = [aggregate_perplexity(lp) for lp in paraphrase_lp]
    perplexity_auc = discrimination_auc(novel_ppl, duplicate_ppl)

    per_k_results: dict = {}
    for k in k_values:
        novel_r = [per_token_rarity(lp, k) for lp in novel_lp]
        duplicate_r = [per_token_rarity(lp, k) for lp in duplicate_lp]
        paraphrase_r = [per_token_rarity(lp, k) for lp in paraphrase_lp]
        per_k_results[str(k)] = {
            "auc_novel_vs_duplicate": discrimination_auc(novel_r, duplicate_r),
            "rarity_scores_micros": {
                "novel": [to_micros(v) for v in novel_r],
                "duplicate": [to_micros(v) for v in duplicate_r],
                "paraphrase": [to_micros(v) for v in paraphrase_r],
            },
        }

    return {
        "candidate_id": candidate,
        "corpus_sha256": corpus.corpus_sha256,
        "model_sha_or_revision": model_revision,
        "k_values": list(k_values),
        "slice_counts": {
            "novel": len(corpus.novel),
            "duplicate": len(corpus.duplicate),
            "paraphrase": len(corpus.paraphrase),
        },
        "perplexity_baseline": {
            "auc_novel_vs_duplicate": perplexity_auc,
        },
        "per_k_results": per_k_results,
        "elapsed_seconds": elapsed,
        "tokens_per_second": (total_tokens / elapsed) if elapsed > 0 else 0.0,
    }


# ---------------------------------------------------------------------------
# CLI.
# ---------------------------------------------------------------------------


def _default_corpus() -> Path:
    return Path("scripts/operator/fixtures/corpus-a26.tar.zst")


def main(argv: List[str]) -> int:
    p = argparse.ArgumentParser(
        description="A.5a per-token rarity pre-flight (multi-K AUC sweep).",
    )
    p.add_argument(
        "--candidate",
        required=True,
        help="candidate id (matches a model dir under ~/models/<id>/ or an HF repo id)",
    )
    p.add_argument(
        "--corpus",
        type=Path,
        default=_default_corpus(),
        help="path to bake-off corpus .tar.zst (default: A2.6 committed fixture)",
    )
    p.add_argument(
        "--k-values",
        default="4,8,16,32",
        help='comma-separated K values for the rarity sweep (default "4,8,16,32")',
    )
    p.add_argument(
        "--report-out",
        type=Path,
        required=True,
        help="path to write the JSON report",
    )
    p.add_argument(
        "--mock-logprobs-seed",
        type=int,
        default=None,
        help=(
            "if set, skip transformers/torch and synthesize deterministic "
            "logprobs from sha256(text|seed). Smoke-test path only."
        ),
    )
    args = p.parse_args(argv)

    try:
        k_values = parse_k_values(args.k_values)
    except ValueError as e:
        print(f"--k-values: {e}", file=sys.stderr)
        return 2

    if not args.corpus.is_file():
        print(f"corpus not found: {args.corpus}", file=sys.stderr)
        return 2

    corpus = load_corpus(args.corpus)

    print("=== A.5a rarity pre-flight ===", file=sys.stderr)
    print(f"Candidate: {args.candidate}", file=sys.stderr)
    print(f"Corpus: {corpus.corpus_sha256}", file=sys.stderr)
    print(
        f"Slices: novel={len(corpus.novel)} "
        f"duplicate={len(corpus.duplicate)} "
        f"paraphrase={len(corpus.paraphrase)}",
        file=sys.stderr,
    )
    print(f"K values: {k_values}", file=sys.stderr)

    if args.mock_logprobs_seed is not None:
        score_fn, model_revision = _load_mock_scorer(args.mock_logprobs_seed)
        print(
            f"Mock scorer active (seed={args.mock_logprobs_seed}); "
            "transformers NOT loaded",
            file=sys.stderr,
        )
    else:
        score_fn, model_revision = _load_real_scorer(args.candidate)

    report = run_preflight(
        candidate=args.candidate,
        corpus=corpus,
        k_values=k_values,
        score_fn=score_fn,
        model_revision=model_revision,
    )

    args.report_out.parent.mkdir(parents=True, exist_ok=True)
    args.report_out.write_text(json.dumps(report, indent=2, sort_keys=True))

    # Summary to stderr — hash-only / counts only.
    print(file=sys.stderr)
    print(
        f"Perplexity baseline AUC (novel vs duplicate): "
        f"{report['perplexity_baseline']['auc_novel_vs_duplicate']:.4f}",
        file=sys.stderr,
    )
    for k in k_values:
        block = report["per_k_results"][str(k)]
        print(
            f"K={k}: rarity AUC = {block['auc_novel_vs_duplicate']:.4f}",
            file=sys.stderr,
        )
    print(
        f"Elapsed: {report['elapsed_seconds']:.2f}s "
        f"({report['tokens_per_second']:.1f} tok/s)",
        file=sys.stderr,
    )
    print(f"Report: {args.report_out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
