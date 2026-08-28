#!/usr/bin/env python3
"""Build an agent-traces bake-off corpus tarball.

Compatible with the `trace-commons-gate-calibrate bake-off` loader
(`crates/trace-commons-server/src/bin/gate_calibrate/
bakeoff_corpus.rs`).

WHAT CHANGED, AND WHY (#204, #205)
----------------------------------

The A2.6 version of this script built the novel slice from agent
traces and reused the duplicate and paraphrase slices verbatim from a
separate `corpus-wiki.tar.zst`. Novel and duplicate therefore came
from different source populations, and every property that tracks
source separated the classes: paragraph count scored AUC 1.000 on the
resulting corpus, because all 300 duplicate files had exactly one
paragraph and every novel file had between 7 and 163. Six no-model
measures beat the model that corpus was used to select. What the
bake-off measured was a source-format detector.

That path is gone. Both slices are now drawn from the same
population and differ only in novelty:

    novel[i]      = an agent trace
    duplicate[i]  = a transformed version of THAT SAME trace

so source, format, subject matter and length distribution are held
constant by construction rather than by hope. #204's second finding
is why length is enforced rather than assumed: the A2.6 paraphrase
slice held the source constant and still left a length confound just
as strong, because 299 of its 300 paraphrases were shorter than their
original (median length ratio 0.282). `--length-band` rejects any
pair the transform did not keep the length of.

Two transforms ship:

  * `shuffle-paragraphs` (default) -- model-free and deterministic.
    Permutes the trace's paragraphs. The multiset of paragraphs, the
    byte count and the line count are preserved exactly, so it needs
    no GPU, no weights and no network. Be honest about what this
    buys: because it is byte-preserving, the trivial-measure battery
    is satisfied by construction on this transform, not by luck. It
    is a structural control -- a duplicate that is genuinely the same
    content -- and it is the right shape for a redundancy gate, but
    it is not a semantic-difficulty benchmark.
  * `external` -- back-translation or paraphrase through a helper
    subprocess (`scripts/operator/bakeoff_paraphrase.py` implements
    the contract). Reads `{"original": ...}` JSONL on stdin, writes
    `{"original": ..., "paraphrase": ...}` JSONL on stdout. Pairs
    outside `--length-band` are rejected and backfilled from the
    pool; if the transform cannot produce enough length-matched
    pairs the build fails rather than emitting a length-confounded
    corpus.

THE VALIDITY GATE
-----------------

After packing, and before the tarball is moved into place, the
trivial-measure battery runs against it via
`trace-commons-gate-calibrate corpus-validity`. Paragraph count,
line count, distinct word count, UTF-8 byte count, whitespace word
count and mean word length must all land near AUC 0.5 under the
repository's own tie convention. A corpus a single integer can
classify is not written. The gate is fail-closed: if the binary
cannot be found, no corpus is produced.

SOURCES
-------

`--source` pulls sessions from a HuggingFace dataset (default
`jedisct1/agent-traces-swival`, MIT-licensed, produced by the Swival
harness). `--novel-corpus` instead takes the novel slice from an
existing corpus tarball, which is how a corrected corpus is built
over the same traces an earlier one used -- no network, and the
novel slice stays comparable to the archived bake-off.

Swival schema (authoritative):

    The dataset is a collection of `*.jsonl` files at the repo root.
    Each file is one session (~3,330 sessions total). Each line in a
    file is one event in that session. Event rows do NOT share a
    common schema across files -- columns drift per session, so
    `datasets.load_dataset(..., streaming=True)` raises a CastError
    when it tries to enforce a single Arrow schema across the file
    set. Avoid that path; download the raw `.jsonl` files and parse
    them by hand.

    Event row fields observed in practice include `uuid`,
    `parentUuid`, `sessionId`, `harness`, `type`, `content`, and a
    nested `message` object whose `content` is either a string or a
    list of `{type, text, ...}` chunks.

Tarball layout (matches the Rust loader exactly):

    manifest.json                {"version":1,
                                  "novel_sha256":"sha256:...",
                                  "duplicate_sha256":"sha256:...",
                                  "paraphrase_sha256":"sha256:..."}
    novel/novel-NNNN.txt         one entry per trace, UTF-8
    duplicate/dup-NNNN.txt       transform of novel-NNNN.txt
    paraphrase/paraphrase.jsonl  the same pairs, as
                                 {"original","paraphrase"}

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
import glob
import hashlib
import io
import json
import os
import random
import shutil
import subprocess
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

# Maximum tolerated relative word-count difference between a trace and its
# transformed duplicate. The A2.6 paraphrase slice sat at a median ratio of
# 0.282 -- a length confound strong enough that byte count out-scored the
# selected model by 0.24 AUC on the source-controlled pair (#204).
DEFAULT_LENGTH_BAND = 0.10

# How many extra candidates to draw so rejected pairs can be backfilled.
DEFAULT_OVERSAMPLE = 3

DEFAULT_VALIDITY_BINARY = "trace-commons-gate-calibrate"

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
# zstd: module when present, CLI otherwise
# ---------------------------------------------------------------------------


def _zstd_module():
    try:
        import zstandard  # type: ignore

        return zstandard
    except ImportError:
        return None


def zstd_compress(raw: bytes) -> bytes:
    """Compress with the `zstandard` module, falling back to the `zstd` CLI.

    `build-bakeoff-corpus.sh` already requires the `zstd` binary, so the
    fallback adds no dependency the operator flow did not have; it only lets
    this script run on hosts without the Python package.
    """
    mod = _zstd_module()
    if mod is not None:
        return mod.ZstdCompressor().compress(raw)
    exe = shutil.which("zstd")
    if exe is None:
        raise bail("zstd_unavailable_no_module_and_no_binary")
    proc = subprocess.run([exe, "-q", "-c", "-"], input=raw, stdout=subprocess.PIPE)
    if proc.returncode != 0:
        raise bail(f"zstd_compress_failed rc={proc.returncode}")
    return proc.stdout


def zstd_decompress(path: Path) -> bytes:
    """Decompress a .zst file to bytes, module first then `zstd` CLI."""
    mod = _zstd_module()
    if mod is not None:
        with open(path, "rb") as fh:
            return mod.ZstdDecompressor().stream_reader(fh).read()
    exe = shutil.which("zstd")
    if exe is None:
        raise bail("zstd_unavailable_no_module_and_no_binary")
    proc = subprocess.run([exe, "-d", "-q", "-c", str(path)], stdout=subprocess.PIPE)
    if proc.returncode != 0:
        raise bail(f"zstd_decompress_failed rc={proc.returncode}")
    return proc.stdout


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
    """Deterministically sample exactly `count` in-band traces.

    Thin wrapper over `collect_pool` kept for callers that want the slice
    itself rather than a pool to draw duplicates from.
    """
    return collect_pool(session_paths, formatter, count, seed, pool_cap)[:count]


def collect_pool(
    session_paths: Iterable[Path],
    formatter,
    count: int,
    seed: int,
    pool_cap: int,
) -> list[str]:
    """Filter session traces to 200-2000 words and shuffle up to `pool_cap`.

    Each yielded path is one swival session `.jsonl`. We flatten each
    session into one trace via `formatter`, then filter by word
    count, accumulate up to `pool_cap` entries, and draw them in a
    seeded order. Returning more than `count` gives `build_pairs`
    room to backfill pairs a transform failed to length-match,
    without materializing every session in memory at once.
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
    return rng.sample(pool, min(len(pool), pool_cap))


def read_novel_slice(tarball: Path) -> list[str]:
    """Read the `novel/` slice out of an existing corpus tarball.

    Sorted by filename, matching the Rust loader's order, so a corpus built
    this way lines up with per-trace scores captured against the source
    corpus.
    """
    step("novel_corpus_open", tarball_label=_short_label(str(tarball)))
    try:
        raw = zstd_decompress(tarball)
        tf = tarfile.open(fileobj=io.BytesIO(raw), mode="r:")
        bodies: dict[str, bytes] = {}
        for member in tf:
            if not member.isfile():
                continue
            name = member.name.lstrip("./")
            if name.startswith("novel/"):
                fh = tf.extractfile(member)
                if fh is None:
                    continue
                bodies[name] = fh.read()
    except SystemExit:
        raise
    except Exception as exc:  # noqa: BLE001
        raise bail(f"novel_corpus_read_failed detail={type(exc).__name__}")
    if not bodies:
        raise bail("novel_slice_empty_in_source")
    ordered = [bodies[k] for k in sorted(bodies)]
    return [b.decode("utf-8", errors="replace") for b in ordered]


# ---------------------------------------------------------------------------
# Duplicate construction: same source, transformed
# ---------------------------------------------------------------------------


def shuffle_paragraphs(text: str, seed: int) -> str:
    """Permute a trace's paragraphs, preserving every trivial measure exactly.

    Splitting on `"\\n\\n"` and rejoining with `"\\n\\n"` moves no bytes and
    changes no newline count, so byte count, line count, paragraph count, word
    count, distinct word count and mean word length are all identical to the
    original. That is the point: the duplicate differs from the novel trace in
    content order alone, which is what a redundancy gate is supposed to catch
    and what no structural measure can see.

    A permutation can come back as the identity; we retry a few times before
    accepting it, because an unchanged body is a weaker duplicate than a
    reordered one, not because an exact duplicate would be invalid.
    """
    blocks = text.split("\n\n")
    if len(blocks) < 2:
        return text
    rng = random.Random(seed)
    for _ in range(8):
        shuffled = blocks[:]
        rng.shuffle(shuffled)
        out = "\n\n".join(shuffled)
        if out != text:
            return out
    return "\n\n".join(blocks)


def run_external_transform(cmd: str, originals: list[str]) -> dict[str, str]:
    """Pipe originals through a paraphrase/back-translation helper.

    Contract matches `scripts/operator/bakeoff_paraphrase.py`: JSONL
    `{"original": ...}` on stdin, JSONL `{"original": ..., "paraphrase": ...}`
    on stdout. The helper's stderr is passed through so its own label-only
    diagnostics reach the operator; we never echo trace text ourselves.
    """
    payload = "".join(json.dumps({"original": o}) + "\n" for o in originals)
    step("external_transform_begin", rows=len(originals))
    try:
        proc = subprocess.run(
            cmd,
            shell=True,
            input=payload.encode("utf-8"),
            stdout=subprocess.PIPE,
        )
    except Exception as exc:  # noqa: BLE001
        raise bail(f"external_transform_spawn_failed detail={type(exc).__name__}")
    if proc.returncode != 0:
        raise bail(f"external_transform_failed rc={proc.returncode}")

    out: dict[str, str] = {}
    for lineno, line in enumerate(proc.stdout.decode("utf-8", errors="replace").splitlines()):
        if not line.strip():
            continue
        try:
            obj = json.loads(line)
            out[obj["original"]] = obj["paraphrase"]
        except Exception:  # noqa: BLE001 — label only, never the row body
            raise bail(f"external_transform_bad_row row={lineno}")
    step("external_transform_done", rows=len(out))
    return out


def length_matched(original: str, transformed: str, band: float) -> bool:
    """True when the transform kept the trace's length inside `band`.

    Relative word-count difference. #204's diagnosis of the A2.6 paraphrase
    slice is the reason this is enforced rather than assumed: a paraphraser
    that systematically shortens its input hands back a corpus separable by
    byte count, with the source confound removed and a length confound left
    in its place.
    """
    ow = len(original.split())
    if ow == 0:
        return False
    tw = len(transformed.split())
    return abs(tw - ow) / ow <= band


def build_pairs(
    pool: list[str],
    count: int,
    transform: str,
    transform_cmd: str | None,
    seed: int,
    band: float,
) -> list[tuple[str, str]]:
    """Return `count` (novel, duplicate) pairs drawn from one population.

    Pairs whose duplicate falls outside the length band are dropped and
    backfilled from the rest of the pool. Running out is a hard failure: a
    short corpus is recoverable, a length-confounded one is not.
    """
    if transform == "shuffle-paragraphs":
        pairs = []
        for i, original in enumerate(pool):
            dup = shuffle_paragraphs(original, seed + i)
            if length_matched(original, dup, band):
                pairs.append((original, dup))
            if len(pairs) >= count:
                break
    elif transform == "external":
        if not transform_cmd:
            raise bail("transform_cmd_required_for_external")
        mapped = run_external_transform(transform_cmd, pool)
        pairs = []
        for original in pool:
            dup = mapped.get(original)
            if dup is None or not dup.strip():
                continue
            if length_matched(original, dup, band):
                pairs.append((original, dup))
            if len(pairs) >= count:
                break
    else:
        raise bail(f"unknown_transform label={transform}")

    if len(pairs) < count:
        raise bail(
            f"insufficient_length_matched_pairs accepted={len(pairs)} "
            f"target={count} band={band}"
        )
    return pairs[:count]


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


def pack_tarball(staging: Path, output: Path) -> None:
    """Pack the staging tree, byte-reproducibly.

    Entry metadata is normalised -- zero mtime, zero uid/gid, fixed mode, no
    owner names -- because `tarfile.add` would otherwise stamp the build host
    and the wall clock into the archive, and a corpus whose sha256 changes on
    every run cannot be cited in a report. The slice hashes in `manifest.json`
    cover file bodies only, so this is the only thing standing behind a stable
    tarball digest.
    """
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.GNU_FORMAT) as tf:
        for entry in sorted(staging.rglob("*")):
            rel = entry.relative_to(staging)
            info = tarfile.TarInfo("./" + str(rel).replace(os.sep, "/"))
            info.mtime = 0
            info.uid = 0
            info.gid = 0
            info.uname = ""
            info.gname = ""
            if entry.is_dir():
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                tf.addfile(info)
                continue
            body = entry.read_bytes()
            info.type = tarfile.REGTYPE
            info.mode = 0o644
            info.size = len(body)
            tf.addfile(info, io.BytesIO(body))
    with open(output, "wb") as out_fh:
        out_fh.write(zstd_compress(buf.getvalue()))


# ---------------------------------------------------------------------------
# The validity gate
# ---------------------------------------------------------------------------


def run_validity_gate(binary: str, corpus: Path, ceiling: float) -> None:
    """Refuse a corpus a trivial measure can classify.

    Shells out to `trace-commons-gate-calibrate corpus-validity`, which runs
    the six preregistered no-model measures through the repository's own
    `discrimination_auc`. Fail-closed: a missing binary produces no corpus,
    because a corpus nobody checked is exactly what #204 is about.
    """
    exe = shutil.which(binary) or (binary if Path(binary).exists() else None)
    if exe is None:
        raise bail(f"validity_gate_binary_missing label={_short_label(binary)}")
    step("validity_gate_begin", ceiling=ceiling)
    proc = subprocess.run(
        [
            exe,
            "corpus-validity",
            "--corpus",
            str(corpus),
            "--ceiling",
            str(ceiling),
        ],
        stdout=subprocess.PIPE,
    )
    # The battery's table is counts, ranges and AUCs only -- no trace text --
    # so it is safe to surface, and it is the evidence the corpus is sound.
    sys.stderr.write(proc.stdout.decode("utf-8", errors="replace"))
    if proc.returncode != 0:
        raise bail("corpus_failed_trivial_measure_battery")
    step("validity_gate_passed")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="build-agent-traces-corpus.py",
        description=(
            "Build a bake-off corpus whose novel and duplicate slices come "
            "from the same population and differ only in novelty (#204)."
        ),
    )
    p.add_argument(
        "--source",
        default=DEFAULT_SOURCE,
        help=f"HuggingFace dataset id (default: {DEFAULT_SOURCE})",
    )
    p.add_argument(
        "--novel-corpus",
        type=Path,
        default=None,
        help=(
            "Take the novel slice from an existing corpus tarball instead of "
            "downloading --source. Use this to rebuild a corrected corpus "
            "over the same traces an earlier corpus used."
        ),
    )
    p.add_argument(
        "--format",
        choices=sorted(FORMATS.keys()),
        default="swival",
        help="Row-to-text mapping (default: swival)",
    )
    p.add_argument(
        "--transform",
        choices=["shuffle-paragraphs", "external"],
        default="shuffle-paragraphs",
        help=(
            "How the duplicate slice is derived from the novel slice. "
            "shuffle-paragraphs is model-free and length-exact; external "
            "pipes through --transform-cmd (back-translation or paraphrase)."
        ),
    )
    p.add_argument(
        "--transform-cmd",
        default=None,
        help=(
            "Shell command implementing the paraphrase contract "
            '(JSONL {"original"} in, {"original","paraphrase"} out). '
            "Required with --transform external."
        ),
    )
    p.add_argument(
        "--length-band",
        type=float,
        default=DEFAULT_LENGTH_BAND,
        help=(
            "Maximum relative word-count difference between a trace and its "
            f"duplicate (default: {DEFAULT_LENGTH_BAND})."
        ),
    )
    p.add_argument(
        "--count",
        type=int,
        default=DEFAULT_COUNT,
        help=f"Pair count (default: {DEFAULT_COUNT})",
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
            f"0 means count*{DEFAULT_OVERSAMPLE} (default)."
        ),
    )
    p.add_argument(
        "--validity-binary",
        default=os.environ.get("TRACE_COMMONS_GATE_CALIBRATE", DEFAULT_VALIDITY_BINARY),
        help=(
            "Path to trace-commons-gate-calibrate, used to run the "
            "trivial-measure battery before the corpus is written."
        ),
    )
    p.add_argument(
        "--validity-ceiling",
        type=float,
        default=0.15,
        help="Maximum tolerated |auc-0.5| for any trivial measure (default: 0.15).",
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

    if not str(args.out).endswith(".tar.zst"):
        raise bail("output_must_end_with_tar_zst")
    if args.length_band < 0:
        raise bail("length_band_must_be_non_negative")

    formatter = FORMATS[args.format]
    pool_cap = (
        args.pool_cap
        if args.pool_cap > 0
        else max(args.count * DEFAULT_OVERSAMPLE, args.count)
    )

    step(
        "begin",
        format=args.format,
        transform=args.transform,
        count=args.count,
        seed=args.seed,
        pool_cap=pool_cap,
        length_band=args.length_band,
    )

    # 1. Assemble the candidate pool -- one population, one format.
    if args.novel_corpus is not None:
        if not args.novel_corpus.exists():
            raise bail("novel_corpus_missing")
        pool = read_novel_slice(args.novel_corpus)
        if len(pool) < args.count:
            raise bail(f"insufficient_novel_slice pool={len(pool)} target={args.count}")
        step("pool_from_corpus", pool=len(pool))
    else:
        pool = collect_pool(
            session_paths=iter_session_files(args.source),
            formatter=formatter,
            count=args.count,
            seed=args.seed,
            pool_cap=pool_cap,
        )
        step("pool_from_dataset", pool=len(pool))

    # 2. Derive each duplicate from its own novel trace.
    pairs = build_pairs(
        pool=pool,
        count=args.count,
        transform=args.transform,
        transform_cmd=args.transform_cmd,
        seed=args.seed,
        band=args.length_band,
    )
    step("pairs_built", pairs=len(pairs))

    # 3. Stage, hash, manifest, pack -- to a temporary path.
    with tempfile.TemporaryDirectory(prefix="bakeoff-corpus-") as tmpdir:
        staging = Path(tmpdir) / "staging"
        novel_dir = staging / "novel"
        duplicate_dir = staging / "duplicate"
        paraphrase_dir = staging / "paraphrase"

        novel_bodies = [o.encode("utf-8") for o, _ in pairs]
        duplicate_bodies = [d.encode("utf-8") for _, d in pairs]
        write_slice_files(novel_dir, "novel", novel_bodies)
        write_slice_files(duplicate_dir, "dup", duplicate_bodies)

        # The paraphrase slice carries the same pairs, so the paraphrase
        # metric and the discrimination metric are computed over one
        # construction rather than two unrelated ones.
        paraphrase_dir.mkdir(parents=True, exist_ok=True)
        paraphrase_jsonl = "".join(
            json.dumps({"original": o, "paraphrase": d}) + "\n" for o, d in pairs
        ).encode("utf-8")
        with open(paraphrase_dir / "paraphrase.jsonl", "wb") as fh:
            fh.write(paraphrase_jsonl)

        manifest = {
            "version": MANIFEST_VERSION,
            "novel_sha256": sha256_label_of_chunks(novel_bodies),
            "duplicate_sha256": sha256_label_of_chunks(duplicate_bodies),
            "paraphrase_sha256": sha256_label_of_chunks([paraphrase_jsonl]),
        }
        with open(staging / "manifest.json", "w", encoding="utf-8") as fh:
            json.dump(manifest, fh, separators=(",", ":"))
            fh.write("\n")

        step(
            "manifest_emitted",
            novel_sha=manifest["novel_sha256"][:19],
            duplicate_sha=manifest["duplicate_sha256"][:19],
            paraphrase_sha=manifest["paraphrase_sha256"][:19],
        )

        pending = Path(tmpdir) / "pending.tar.zst"
        pack_tarball(staging, pending)

        # 4. Gate. Nothing is written to --out until the battery passes.
        run_validity_gate(args.validity_binary, pending, args.validity_ceiling)

        args.out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(pending, args.out)

    tar_sha = sha256_label_of_file(args.out)
    print(f"BakeoffAgentTracesOK output_sha256={tar_sha}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
