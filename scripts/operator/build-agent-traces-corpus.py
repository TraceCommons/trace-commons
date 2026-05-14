#!/usr/bin/env python3
"""Build an agent-traces bake-off corpus tarball for A2.6.

This is the A2.6a operator addendum: it builds a corpus tarball
compatible with the existing `tracedao-gate-calibrate bake-off`
binary's loader (`crates/tracedao-server/src/bin/gate_calibrate/
bakeoff_corpus.rs`). The novel slice is swapped from OASST2 chat
(A2.3c/A2.4) to agent-traces drawn from a HuggingFace dataset; the
duplicate slice and the paraphrase slice are reused verbatim from an
existing A2.4-era `corpus-wiki.tar.zst`.

Default source dataset is `jedisct1/agent-traces-swival` — MIT-
licensed agent traces produced by the Swival harness.

Swival schema (authoritative):

    The dataset is a collection of `*.jsonl` files at the repo root.
    Each file is one session (~3,330 sessions total). Each line in a
    file is one event in that session. Event rows do NOT share a
    common schema across files — columns drift per session, so
    `datasets.load_dataset(..., streaming=True)` raises a CastError
    when it tries to enforce a single Arrow schema across the file
    set. Avoid that path; download the raw `.jsonl` files and parse
    them by hand.

    Event row fields observed in practice include `uuid`,
    `parentUuid`, `sessionId`, `harness`, `type`, `content`, and a
    nested `message` object whose `content` is either a string or a
    list of `{type, text, ...}` chunks. The narrative-field schema
    used by an earlier draft (`title`, `severity`, `proof`,
    `fix_outline`, etc.) does NOT exist on disk and was never
    correct.

This script flattens each session into one trace by concatenating
every non-empty text snippet found on `message.content` (string or
chunk list) and on the top-level `content` field, joined by double
newlines. The result is a multi-paragraph body that resembles the
kind of trace Trace Commons is intended to gate.

The `--format` flag selects the session-to-text mapping. v1 ships
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
import glob
import hashlib
import io
import json
import os
import random
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
# Session -> text mappings
# ---------------------------------------------------------------------------


def _extract_event_text(event: dict) -> list[str]:
    """Pull every non-empty text snippet out of one swival event row.

    Looks at `message.content` (string OR list-of-chunks with `text`
    fields) and the top-level `content` field. Returns trimmed
    snippets in observed order; the caller joins them with blank
    lines so they read as one trace.
    """
    parts: list[str] = []

    msg = event.get("message")
    if isinstance(msg, dict):
        c = msg.get("content", "")
        if isinstance(c, str):
            s = c.strip()
            if s:
                parts.append(s)
        elif isinstance(c, list):
            for chunk in c:
                if isinstance(chunk, dict):
                    t = chunk.get("text")
                    if isinstance(t, str):
                        s = t.strip()
                        if s:
                            parts.append(s)

    c2 = event.get("content")
    if isinstance(c2, str):
        s = c2.strip()
        if s:
            parts.append(s)

    return parts


def swival_session_to_text(session_lines: Iterable[str]) -> str:
    """Concat every event's content fields into one prose trace.

    `session_lines` is the raw line iterator of a swival session's
    `.jsonl` file. Lines that fail to parse as JSON are skipped
    silently (per the hash-only logging convention; malformed event
    rows do happen in the wild and are not operator-actionable).
    """
    parts: list[str] = []
    for line in session_lines:
        try:
            event = json.loads(line)
        except Exception:  # noqa: BLE001 — malformed line, skip
            continue
        if not isinstance(event, dict):
            continue
        parts.extend(_extract_event_text(event))
    return "\n\n".join(parts).strip()


FORMATS = {
    "swival": swival_session_to_text,
}


# ---------------------------------------------------------------------------
# Dataset loading
# ---------------------------------------------------------------------------


def iter_session_files(source: str) -> Iterator[Path]:
    """Download the dataset's `*.jsonl` files and yield their paths.

    Uses `huggingface_hub.snapshot_download` to grab every `.jsonl`
    file at the dataset root. Each file is one session. Yields in
    sorted filename order so sampling at a fixed seed is
    reproducible.
    """
    try:
        from huggingface_hub import snapshot_download
    except ImportError as exc:
        raise bail(f"huggingface_hub_package_missing detail={type(exc).__name__}")

    step("dataset_open", source_label=_short_label(source))
    try:
        local_dir = snapshot_download(
            repo_id=source,
            repo_type="dataset",
            allow_patterns=["*.jsonl"],
        )
    except Exception as exc:  # noqa: BLE001 — surface label only
        raise bail(f"dataset_load_failed detail={type(exc).__name__}")

    paths = sorted(glob.glob(os.path.join(local_dir, "*.jsonl")))
    if not paths:
        raise bail("dataset_no_jsonl_files")
    step("dataset_sessions_listed", sessions=len(paths))
    for p in paths:
        yield Path(p)


def _short_label(value: str) -> str:
    """Return a short hash-label of `value` so we don't echo raw ids."""
    h = hashlib.sha256(value.encode("utf-8")).hexdigest()[:12]
    return f"sha256:{h}"


# ---------------------------------------------------------------------------
# Novel-slice extraction
# ---------------------------------------------------------------------------


def collect_novel_texts(
    session_paths: Iterable[Path],
    formatter,
    count: int,
    seed: int,
    pool_cap: int,
) -> list[str]:
    """Filter session traces to 200-2000 words and deterministically sample `count`.

    Each yielded path is one swival session `.jsonl`. We flatten each
    session into one trace via `formatter`, then filter by word
    count, accumulate up to `pool_cap` entries, and draw `count` of
    them with the given seed. This gives reproducible output without
    materializing every session in memory at once.
    """
    if count <= 0:
        raise bail("count_must_be_positive")
    pool: list[str] = []
    seen = 0
    for path in session_paths:
        seen += 1
        try:
            with open(path, "r", encoding="utf-8", errors="replace") as fh:
                text = formatter(fh)
        except Exception:  # noqa: BLE001 — skip malformed sessions silently
            continue
        if not text:
            continue
        words = text.split()
        if not (MIN_WORDS <= len(words) <= MAX_WORDS):
            continue
        pool.append(text)
        if len(pool) >= pool_cap:
            break
        if seen % 250 == 0:
            step("dataset_scan", scanned=seen, pool=len(pool))

    step("dataset_scan_done", scanned=seen, pool=len(pool))
    if len(pool) < count:
        raise bail(f"insufficient_filtered_sessions pool={len(pool)} target={count}")

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
    session_paths = iter_session_files(args.source)
    novel_texts = collect_novel_texts(
        session_paths=session_paths,
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
