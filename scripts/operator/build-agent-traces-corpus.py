#!/usr/bin/env python3
"""Build an agent-traces bake-off corpus tarball for A2.6.

This is the A2.6a operator addendum: it builds a corpus tarball
compatible with the existing `tracedao-gate-calibrate bake-off`
binary's loader (`crates/tracedao-server/src/bin/gate_calibrate/
bakeoff_corpus.rs`). The novel slice is swapped from OASST2 chat
(A2.3c/A2.4) to agent-traces drawn from a HuggingFace dataset; the
duplicate slice and the paraphrase slice are reused verbatim from an
existing A2.4-era `corpus-wiki.tar.zst`.

Default source dataset is `jedisct1/agent-traces-swival` — 33.7k MIT-
licensed OSS security-audit traces produced by the Swival agent. Each
row is mapped to a single prose body that joins the trace's
human-readable narrative fields:

    novel_text = "\\n\\n".join([
        row["title"],
        row["severity"] + " " + row["finding_type"],
        "\\n".join(row["preconditions"]),
        "\\n".join(row["proof"]),
        row["fix_outline"],
        (row["source_code"] or "")[:1000],
    ]).strip()

The `--format` flag selects the row-to-text mapping. v1 ships
"swival" only; additional dataset formats can be added without
changing the tarball contract.

Tarball layout (matches the Rust loader exactly):

    manifest.json                {"version":1,
                                  "novel_sha256":"sha256:...",
                                  "duplicate_sha256":"sha256:...",
                                  "paraphrase_sha256":"sha256:..."}
    novel/novel-NNNN.txt         one entry per file, UTF-8
    duplicate/dup-NNNN.txt       reused from --duplicate-corpus
    paraphrase/paraphrase.jsonl  reused from --duplicate-corpus

Hash convention:
  * novel + duplicate slice sha256 is over the concatenated raw bytes
    of every regular file in the slice directory, sorted by filename
    (matches Rust `read_text_slice` / `sha256_label`).
  * paraphrase slice sha256 is over the raw bytes of paraphrase.jsonl.

This script never logs raw trace bodies, contributor identity, or
operator-secret material. Step lines are label-only.

Error convention: `BakeoffAgentTracesFailure: <label>`.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import io
import json
import os
import random
import re
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import Iterable, Iterator

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MANIFEST_VERSION = 1
DEFAULT_SOURCE = "jedisct1/agent-traces-swival"
DEFAULT_COUNT = 300
DEFAULT_SEED = 42
MIN_WORDS = 200
MAX_WORDS = 2000
SOURCE_CODE_CAP = 1000

# ---------------------------------------------------------------------------
# Errors / logging helpers
# ---------------------------------------------------------------------------


def bail(label: str) -> "Exception":
    """Build the standard error. Caller `raise`s it."""
    return SystemExit(f"BakeoffAgentTracesFailure: {label}")


def step(phase: str, **kv: object) -> None:
    """Emit a label-only step line on stderr. Never includes trace bodies."""
    pairs = " ".join(f"{k}={v}" for k, v in kv.items())
    if pairs:
        print(f"BakeoffAgentTracesStep: phase={phase} {pairs}", file=sys.stderr)
    else:
        print(f"BakeoffAgentTracesStep: phase={phase}", file=sys.stderr)


# ---------------------------------------------------------------------------
# Row -> text mappings
# ---------------------------------------------------------------------------


def _coerce_list(value: object) -> list[str]:
    if value is None:
        return []
    if isinstance(value, list):
        return [str(v) for v in value if v is not None]
    if isinstance(value, str):
        return [value]
    return [str(value)]


def swival_row_to_text(row: dict) -> str:
    """Join the human-readable narrative fields of a swival row.

    Swival rows describe a single security-audit finding: a title, a
    severity + finding_type, a list of preconditions, a list of proof
    steps, a fix outline, and the source code under audit. Joining
    these as prose gives a multi-paragraph body that resembles the
    kind of trace Trace Commons is intended to gate.
    """
    parts: list[str] = []
    title = str(row.get("title") or "").strip()
    if title:
        parts.append(title)

    severity = str(row.get("severity") or "").strip()
    finding_type = str(row.get("finding_type") or "").strip()
    if severity or finding_type:
        parts.append(f"{severity} {finding_type}".strip())

    preconditions = _coerce_list(row.get("preconditions"))
    if preconditions:
        parts.append("\n".join(p.strip() for p in preconditions if p.strip()))

    proof = _coerce_list(row.get("proof"))
    if proof:
        parts.append("\n".join(p.strip() for p in proof if p.strip()))

    fix_outline = str(row.get("fix_outline") or "").strip()
    if fix_outline:
        parts.append(fix_outline)

    source_code = str(row.get("source_code") or "")
    if source_code:
        parts.append(source_code[:SOURCE_CODE_CAP])

    return "\n\n".join(p for p in parts if p).strip()


FORMATS = {
    "swival": swival_row_to_text,
}


# ---------------------------------------------------------------------------
# Dataset loading
# ---------------------------------------------------------------------------


def iter_dataset_rows(source: str) -> Iterator[dict]:
    """Yield rows from a HuggingFace dataset id, streaming.

    Uses `datasets.load_dataset(..., streaming=True)` to avoid pulling
    the full dataset into RAM. The default split is `train`.
    """
    try:
        from datasets import load_dataset
    except ImportError as exc:
        raise bail(f"datasets_package_missing detail={type(exc).__name__}")

    step("dataset_open", source_label=_short_label(source))
    try:
        ds = load_dataset(source, split="train", streaming=True)
    except Exception as exc:  # noqa: BLE001 — surface label only
        raise bail(f"dataset_load_failed detail={type(exc).__name__}")

    for row in ds:
        if isinstance(row, dict):
            yield row


def _short_label(value: str) -> str:
    """Return a short hash-label of `value` so we don't echo raw ids."""
    h = hashlib.sha256(value.encode("utf-8")).hexdigest()[:12]
    return f"sha256:{h}"


# ---------------------------------------------------------------------------
# Novel-slice extraction
# ---------------------------------------------------------------------------


def collect_novel_texts(
    rows: Iterable[dict],
    formatter,
    count: int,
    seed: int,
    pool_cap: int,
) -> list[str]:
    """Filter rows to 200-2000 words and deterministically sample `count`.

    The streaming dataset is consumed up to `pool_cap` filtered
    entries; from that pool we draw `count` with the given seed. This
    gives reproducible output without loading the whole 33.7k-row
    dataset into RAM.
    """
    if count <= 0:
        raise bail("count_must_be_positive")
    pool: list[str] = []
    seen = 0
    for row in rows:
        seen += 1
        try:
            text = formatter(row)
        except Exception:  # noqa: BLE001 — skip malformed rows silently
            continue
        if not text:
            continue
        words = text.split()
        if not (MIN_WORDS <= len(words) <= MAX_WORDS):
            continue
        pool.append(text)
        if len(pool) >= pool_cap:
            break
        if seen % 1000 == 0:
            step("dataset_scan", scanned=seen, pool=len(pool))

    step("dataset_scan_done", scanned=seen, pool=len(pool))
    if len(pool) < count:
        raise bail(f"insufficient_filtered_rows pool={len(pool)} target={count}")

    rng = random.Random(seed)
    sampled = rng.sample(pool, count)
    return sampled


# ---------------------------------------------------------------------------
# Duplicate-corpus reuse
# ---------------------------------------------------------------------------


@dataclasses.dataclass
class ReusedSlices:
    duplicate_files: list[tuple[str, bytes]]  # (filename, raw bytes), sorted
    paraphrase_jsonl: bytes


def extract_duplicate_corpus(tarball: Path) -> ReusedSlices:
    """Open an existing corpus-wiki.tar.zst and return its duplicate + paraphrase bytes.

    The novel/ directory of the source tarball is ignored; only the
    duplicate slice (per-file bodies, sorted by filename) and the
    paraphrase slice (single JSONL) are extracted. Hashes are
    recomputed by the caller from these raw bytes so the new manifest
    is internally consistent.
    """
    try:
        import zstandard  # type: ignore
    except ImportError as exc:
        raise bail(f"zstandard_package_missing detail={type(exc).__name__}")

    step("duplicate_corpus_open", tarball_label=_short_label(str(tarball)))
    try:
        with open(tarball, "rb") as fh:
            dctx = zstandard.ZstdDecompressor()
            raw = dctx.stream_reader(fh)
            tf = tarfile.open(fileobj=raw, mode="r|")

            duplicate_files: dict[str, bytes] = {}
            paraphrase_jsonl: bytes | None = None
            for member in tf:
                if not member.isfile():
                    continue
                name = member.name.lstrip("./")
                if name.startswith("duplicate/"):
                    fh2 = tf.extractfile(member)
                    if fh2 is None:
                        continue
                    duplicate_files[name] = fh2.read()
                elif name == "paraphrase/paraphrase.jsonl":
                    fh2 = tf.extractfile(member)
                    if fh2 is None:
                        continue
                    paraphrase_jsonl = fh2.read()
    except Exception as exc:  # noqa: BLE001
        raise bail(f"duplicate_corpus_read_failed detail={type(exc).__name__}")

    if not duplicate_files:
        raise bail("duplicate_slice_empty_in_source")
    if paraphrase_jsonl is None or not paraphrase_jsonl:
        raise bail("paraphrase_slice_missing_in_source")

    ordered = sorted(duplicate_files.items(), key=lambda kv: kv[0])
    return ReusedSlices(
        duplicate_files=[(name.split("/", 1)[1], body) for name, body in ordered],
        paraphrase_jsonl=paraphrase_jsonl,
    )


# ---------------------------------------------------------------------------
# Hashing + tarball packing
# ---------------------------------------------------------------------------


def sha256_label_of_chunks(chunks: Iterable[bytes]) -> str:
    h = hashlib.sha256()
    for c in chunks:
        h.update(c)
    return f"sha256:{h.hexdigest()}"


def sha256_label_of_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def write_slice_files(out_dir: Path, prefix: str, bodies: list[bytes]) -> list[Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    paths: list[Path] = []
    for i, body in enumerate(bodies):
        p = out_dir / f"{prefix}-{i:04d}.txt"
        with open(p, "wb") as fh:
            fh.write(body)
        paths.append(p)
    return paths


def write_reused_duplicate(out_dir: Path, files: list[tuple[str, bytes]]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    for name, body in files:
        with open(out_dir / name, "wb") as fh:
            fh.write(body)


def pack_tarball(staging: Path, output: Path) -> None:
    try:
        import zstandard  # type: ignore
    except ImportError as exc:
        raise bail(f"zstandard_package_missing detail={type(exc).__name__}")

    # Build the inner tar in memory (corpus is small: ~300 entries × <2 KB).
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tf:
        for entry in sorted(staging.rglob("*")):
            rel = entry.relative_to(staging)
            arcname = "./" + str(rel).replace(os.sep, "/")
            tf.add(str(entry), arcname=arcname, recursive=False)
    raw = buf.getvalue()

    cctx = zstandard.ZstdCompressor()
    with open(output, "wb") as out_fh:
        out_fh.write(cctx.compress(raw))


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="build-agent-traces-corpus.py",
        description=(
            "Build an A2.6 bake-off corpus tarball with an agent-traces "
            "novel slice and the duplicate + paraphrase slices reused "
            "from an existing A2.4 corpus tarball."
        ),
    )
    p.add_argument(
        "--source",
        default=DEFAULT_SOURCE,
        help=f"HuggingFace dataset id (default: {DEFAULT_SOURCE})",
    )
    p.add_argument(
        "--format",
        choices=sorted(FORMATS.keys()),
        default="swival",
        help="Row-to-text mapping (default: swival)",
    )
    p.add_argument(
        "--duplicate-corpus",
        required=True,
        type=Path,
        help="Path to an existing corpus-wiki.tar.zst to reuse duplicate + paraphrase slices from.",
    )
    p.add_argument(
        "--count",
        type=int,
        default=DEFAULT_COUNT,
        help=f"Novel-slice entry count (default: {DEFAULT_COUNT})",
    )
    p.add_argument(
        "--seed",
        type=int,
        default=DEFAULT_SEED,
        help=f"Deterministic RNG seed for sampling (default: {DEFAULT_SEED})",
    )
    p.add_argument(
        "--pool-cap",
        type=int,
        default=0,
        help=(
            "Cap on the filtered pool size before sampling. "
            "0 means count*10 (default)."
        ),
    )
    p.add_argument(
        "--out",
        required=True,
        type=Path,
        help="Output .tar.zst path (must end with .tar.zst).",
    )
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)

    if args.out.suffix not in (".zst",) or not str(args.out).endswith(".tar.zst"):
        raise bail("output_must_end_with_tar_zst")
    if not args.duplicate_corpus.exists():
        raise bail("duplicate_corpus_missing")

    formatter = FORMATS[args.format]
    pool_cap = args.pool_cap if args.pool_cap > 0 else max(args.count * 10, args.count)

    step(
        "begin",
        format=args.format,
        count=args.count,
        seed=args.seed,
        pool_cap=pool_cap,
    )

    # 1. Pull duplicate + paraphrase slices from the existing tarball.
    reused = extract_duplicate_corpus(args.duplicate_corpus)
    step(
        "duplicate_corpus_loaded",
        duplicate_entries=len(reused.duplicate_files),
        paraphrase_bytes=len(reused.paraphrase_jsonl),
    )

    # 2. Stream the source dataset and assemble the novel slice.
    rows = iter_dataset_rows(args.source)
    novel_texts = collect_novel_texts(
        rows=rows,
        formatter=formatter,
        count=args.count,
        seed=args.seed,
        pool_cap=pool_cap,
    )
    step("novel_slice_assembled", count=len(novel_texts))

    # 3. Stage to disk, hash, manifest, pack.
    with tempfile.TemporaryDirectory(prefix="bakeoff-a26-") as tmpdir:
        staging = Path(tmpdir)
        novel_dir = staging / "novel"
        duplicate_dir = staging / "duplicate"
        paraphrase_dir = staging / "paraphrase"

        novel_bodies = [t.encode("utf-8") for t in novel_texts]
        write_slice_files(novel_dir, "novel", novel_bodies)
        write_reused_duplicate(duplicate_dir, reused.duplicate_files)
        paraphrase_dir.mkdir(parents=True, exist_ok=True)
        with open(paraphrase_dir / "paraphrase.jsonl", "wb") as fh:
            fh.write(reused.paraphrase_jsonl)

        novel_sha = sha256_label_of_chunks(novel_bodies)
        duplicate_sha = sha256_label_of_chunks(b for _, b in reused.duplicate_files)
        paraphrase_sha = sha256_label_of_chunks([reused.paraphrase_jsonl])

        manifest = {
            "version": MANIFEST_VERSION,
            "novel_sha256": novel_sha,
            "duplicate_sha256": duplicate_sha,
            "paraphrase_sha256": paraphrase_sha,
        }
        with open(staging / "manifest.json", "w", encoding="utf-8") as fh:
            json.dump(manifest, fh, separators=(",", ":"))
            fh.write("\n")

        step(
            "manifest_emitted",
            novel_sha=novel_sha[:19],
            duplicate_sha=duplicate_sha[:19],
            paraphrase_sha=paraphrase_sha[:19],
        )

        args.out.parent.mkdir(parents=True, exist_ok=True)
        pack_tarball(staging, args.out)

    tar_sha = sha256_label_of_file(args.out)
    print(f"BakeoffAgentTracesOK output_sha256={tar_sha}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
