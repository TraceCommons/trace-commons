#!/usr/bin/env python3
"""Decode the generated PNGs with an implementation we did not write.

crates/trace-commons-mark rasterizes the mark and encodes the PNG itself,
including a small hand-rolled deflate. Testing that encoder with the encoder's
own assumptions proves very little: a stream can be self-consistent and still be
something no other decoder accepts.

So this decodes with Python's standard-library `zlib` -- a different
implementation of the same specification, maintained by somebody else -- and
compares the result to the raw pixel buffer the renderer produced. Both files
are written by `cargo run -p trace-commons-mark --example emit-verify`.

Standard library only, deliberately. Pillow would be the obvious tool and is not
available on every runner; `zlib` and `struct` are always there, which means
this check can run anywhere the drift check runs.

Usage: verify-png.py <dir-containing-mark-N.png-and-mark-N.rgba>
"""

import struct
import sys
import zlib
from pathlib import Path


def decode_png(data: bytes) -> tuple[int, int, bytes]:
    """Return (width, height, RGBA bytes) for an 8-bit RGBA PNG."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG: bad signature")

    pos = 8
    width = height = None
    idat = bytearray()
    seen_iend = False
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        payload = data[pos + 8 : pos + 8 + length]
        (stated_crc,) = struct.unpack(">I", data[pos + 8 + length : pos + 12 + length])
        actual_crc = zlib.crc32(kind + payload) & 0xFFFFFFFF
        if actual_crc != stated_crc:
            raise ValueError(f"chunk {kind!r} CRC {actual_crc:#x} != {stated_crc:#x}")
        if kind == b"IHDR":
            width, height, depth, colour, comp, filt, interlace = struct.unpack(
                ">IIBBBBB", payload
            )
            if (depth, colour, comp, filt, interlace) != (8, 6, 0, 0, 0):
                raise ValueError("expected 8-bit RGBA, non-interlaced")
        elif kind == b"IDAT":
            idat += payload
        elif kind == b"IEND":
            seen_iend = True
        pos += 12 + length

    if not seen_iend:
        raise ValueError("no IEND chunk")
    if width is None:
        raise ValueError("no IHDR chunk")

    raw = zlib.decompress(bytes(idat))
    stride = width * 4
    if len(raw) != height * (stride + 1):
        raise ValueError(f"decompressed {len(raw)} bytes, expected {height * (stride + 1)}")

    out = bytearray()
    previous = bytearray(stride)
    pos = 0
    for row in range(height):
        filter_type = raw[pos]
        pos += 1
        line = bytearray(raw[pos : pos + stride])
        pos += stride
        if filter_type == 0:
            pass
        elif filter_type == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        else:
            raise ValueError(f"row {row} uses unsupported filter {filter_type}")
        out += line
        previous = line
    return width, height, bytes(out)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verify-png.py <dir>", file=sys.stderr)
        return 2
    directory = Path(sys.argv[1])

    pngs = sorted(directory.glob("mark-*.png"))
    if not pngs:
        print(f"FATAL: no mark-*.png in {directory}", file=sys.stderr)
        print("Generate them: cargo run -p trace-commons-mark --example emit-verify", file=sys.stderr)
        return 1

    failures = 0
    for png in pngs:
        raw_path = png.with_suffix(".rgba")
        if not raw_path.exists():
            print(f"FATAL: {png.name} has no matching {raw_path.name}", file=sys.stderr)
            failures += 1
            continue
        try:
            width, height, decoded = decode_png(png.read_bytes())
        except ValueError as err:
            print(f"FATAL: {png.name}: {err}", file=sys.stderr)
            failures += 1
            continue

        expected = raw_path.read_bytes()
        if decoded != expected:
            print(
                f"FATAL: {png.name}: decoded pixels differ from the renderer's output",
                file=sys.stderr,
            )
            failures += 1
            continue

        # Not "more than one colour". A translucent surface over a gradient has
        # hundreds of colours and no mark on it; a flat tile has one and also no
        # mark. The two bracket inks are what distinguishes the mark being drawn
        # from something being drawn.
        pixels = {decoded[i : i + 4] for i in range(0, len(decoded), 4)}
        # Since the mark converged on the site palette these are the accent and
        # ink rather than two hues. Ink is still worth probing for: the tile is
        # a white card, so opaque black is exactly what a drawn bracket adds.
        accent = bytes((0x00, 0xD4, 0xAA, 0xFF))
        ink = bytes((0x00, 0x00, 0x00, 0xFF))
        missing = [
            name
            for name, colour in (("accent", accent), ("ink", ink))
            if colour not in pixels
        ]
        if missing:
            print(
                f"FATAL: {png.name} ({width}x{height}) has no {', '.join(missing)} bracket",
                file=sys.stderr,
            )
            failures += 1
            continue

        ratio = len(expected) / len(png.read_bytes())
        print(f"{png.name}: {width}x{height} decodes exactly, both brackets, {ratio:.0f}x smaller than raw")

    if failures:
        print(f"{failures} file(s) failed", file=sys.stderr)
        return 1
    print(f"{len(pngs)} PNG(s) verified against the standard library's zlib")
    return 0


if __name__ == "__main__":
    sys.exit(main())
